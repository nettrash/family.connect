//! An animation's canvas, frame after frame, as a viewer would show it — and what becomes of it on
//! its way into a video or onto a sticker (`AnimationFrames.cs`; docs/protocol.md, "Animations":
//! "Each frame is COMPOSITED — drawn as a viewer would show it at that moment, its disposal and
//! blending applied").
//!
//! - [`FrameCanvas`]: ONE COMPOSITOR FOR ALL THREE FORMATS. A GIF image, an APNG frame and a WebP
//!   frame are each a rectangle of pixels with a place on the canvas, a way of meeting what is
//!   there and something to do once its time is up; the formats differ only in how they spell
//!   those, which `animated_pictures` translates.
//! - [`animation_render`]: on white, at the video's size, as BT.709 NV12.
//! - [`AnimationFrames`]: the video's frames in the plan's order.
//! - [`sticker_frames`]: an animated STICKER's frames, kept transparent and premultiplied.
//!
//! Where the source frames come from is a [`PixelsOf`]: a GIF's own
//! ([`GifPicture::pixels`](crate::animated_pictures::GifPicture::pixels)), or a standalone still
//! decoded by the platform (`animation_preparing::frame_source`). Everything here is pure
//! arithmetic on byte buffers, and the same on every machine.
//!
//! ON ONE THREAD. Compositing a long animation is real work, and this client does it on the GLib
//! main context. So between source frames [`AnimationFrames::next`] and
//! [`sticker_frames::compose`] yield once to the executor — the window draws and takes input in
//! between, and dropping the future (the C#'s cancel) takes effect there — and they look at the
//! `Cancel` they are given where the C# looked at its token.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use fc_core::rt::{Cancel, LocalBoxFuture};

use crate::animated_pictures::{
    AnimationFrame, AnimationLayout, FrameBlend, FrameDisposal, bgra_len,
};
use crate::animation_timing::AnimationPlan;
use crate::transcode_attempts::OperationCanceled;

/// Source frame `n`'s own rectangle as straight BGRA, `width × height × 4` bytes — the platform's
/// decoder, or a GIF's own. `None` is a frame that could not be decoded.
pub type PixelsOf = Rc<dyn Fn(usize) -> LocalBoxFuture<'static, Option<Vec<u8>>>>;

/// A frame's pixels are not the size of its rectangle (the C#'s `ArgumentException`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrongSize;

impl std::fmt::Display for WrongSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a frame's pixels are not the size of its rectangle")
    }
}

impl std::error::Error for WrongSize {}

/// An animation's canvas, frame after frame, as a viewer would show it.
///
/// STRAIGHT ALPHA, AND TRANSPARENT TO BEGIN WITH. The canvas starts empty — every browser ignores
/// a GIF's background colour and a WebP's `ANIM` one — and "restore to background" clears a
/// rectangle to empty again. White is put under it only at the end ([`animation_render::on_white`]),
/// so a half-transparent APNG edge drawn over another is blended as the browsers blend it before
/// anything is flattened.
///
/// A disposal is applied when the NEXT frame is drawn, as the formats define it: what
/// [`pixels`](Self::pixels) holds after a [`draw`](Self::draw) is exactly what shows while that
/// frame stands.
#[derive(Debug, Clone)]
pub struct FrameCanvas {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
    standing: Option<AnimationFrame>,
    before: Option<Vec<u8>>,
}

impl FrameCanvas {
    /// An empty canvas — `None` for a side that is not positive, or one too large to count in an
    /// `i32` of bytes (where the C# threw).
    pub fn new(width: i32, height: i32) -> Option<FrameCanvas> {
        if width <= 0 || height <= 0 {
            return None;
        }
        Some(FrameCanvas {
            width,
            height,
            pixels: vec![0; bgra_len(width, height)?],
            standing: None,
            before: None,
        })
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// The canvas as straight BGRA, `width × height × 4`.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The frame before this one disposed of as it asked, then `frame` drawn — its
    /// `width × height × 4` straight-BGRA `pixels`, clipped to the canvas.
    pub fn draw(&mut self, frame: &AnimationFrame, pixels: &[u8]) -> Result<(), WrongSize> {
        if frame.pixel_bytes() != Some(pixels.len()) {
            return Err(WrongSize);
        }
        if let Some(last) = self.standing {
            match last.disposal {
                FrameDisposal::Background => self.clear(&last),
                FrameDisposal::Previous => {
                    if let Some(saved) = self.before.take() {
                        self.restore(&last, &saved);
                    }
                }
                FrameDisposal::None => {}
            }
        }
        self.before = (frame.disposal == FrameDisposal::Previous).then(|| self.save(frame));
        let (left, top, right, bottom) = self.clip(frame);
        let canvas_width = self.width as usize;
        let frame_width = frame.width as usize;
        for y in top..bottom {
            let mut source = ((y - frame.y) as usize * frame_width + (left - frame.x) as usize) * 4;
            let mut target = (y as usize * canvas_width + left as usize) * 4;
            for _ in left..right {
                if frame.blend == FrameBlend::Source {
                    self.pixels[target..target + 4].copy_from_slice(&pixels[source..source + 4]);
                } else {
                    FrameCanvas::over(
                        &pixels[source..source + 4],
                        &mut self.pixels[target..target + 4],
                    );
                }
                source += 4;
                target += 4;
            }
        }
        self.standing = Some(*frame);
        Ok(())
    }

    /// `source` drawn over `target`, both one straight-BGRA pixel: Porter-Duff OVER, rounded to the
    /// nearest. An opaque source replaces, an empty one changes nothing.
    pub fn over(source: &[u8], target: &mut [u8]) {
        let sa = i32::from(source[3]);
        if sa == 255 {
            target[..4].copy_from_slice(&source[..4]);
            return;
        }
        if sa == 0 {
            return;
        }
        let da = i32::from(target[3]);
        // Alpha out, times 255: sa·255 + da·(255 − sa).
        let weight = sa * 255 + da * (255 - sa);
        for channel in 0..3 {
            let mixed = i32::from(source[channel]) * sa * 255
                + i32::from(target[channel]) * da * (255 - sa);
            target[channel] = ((mixed + weight / 2) / weight) as u8;
        }
        target[3] = ((weight + 127) / 255) as u8;
    }

    fn clip(&self, frame: &AnimationFrame) -> (i32, i32, i32, i32) {
        (
            frame.x.clamp(0, self.width),
            frame.y.clamp(0, self.height),
            (i64::from(frame.x) + i64::from(frame.width)).clamp(0, i64::from(self.width)) as i32,
            (i64::from(frame.y) + i64::from(frame.height)).clamp(0, i64::from(self.height)) as i32,
        )
    }

    fn clear(&mut self, frame: &AnimationFrame) {
        let (left, top, right, bottom) = self.clip(frame);
        for y in top..bottom {
            let start = ((y * self.width + left) * 4) as usize;
            let end = start + ((right - left).max(0) * 4) as usize;
            self.pixels[start..end].fill(0);
        }
    }

    fn save(&self, frame: &AnimationFrame) -> Vec<u8> {
        let (left, top, right, bottom) = self.clip(frame);
        let row = ((right - left).max(0) * 4) as usize;
        let mut saved = Vec::with_capacity(row * (bottom - top).max(0) as usize);
        for y in top..bottom {
            let start = ((y * self.width + left) * 4) as usize;
            saved.extend_from_slice(&self.pixels[start..start + row]);
        }
        saved
    }

    fn restore(&mut self, frame: &AnimationFrame, saved: &[u8]) {
        let (left, top, right, bottom) = self.clip(frame);
        let row = ((right - left).max(0) * 4) as usize;
        for y in top..bottom {
            let from = (y - top) as usize * row;
            let start = ((y * self.width + left) * 4) as usize;
            self.pixels[start..start + row].copy_from_slice(&saved[from..from + row]);
        }
    }
}

/// What a composited canvas becomes on its way into the encoder: on white, at the video's size, in
/// its pixel format (`AnimationRender`).
pub mod animation_render {
    /// Straight BGRA drawn onto [`BACKGROUND_RGB`](crate::animation_timing::BACKGROUND_RGB) white:
    /// every pixel opaque after.
    pub fn on_white(straight: &[u8]) -> Vec<u8> {
        let mut opaque = vec![0u8; straight.len()];
        let mut at = 0;
        while at + 3 < straight.len() {
            let alpha = u32::from(straight[at + 3]);
            let uncovered = 255 * (255 - alpha);
            for channel in 0..3 {
                opaque[at + channel] =
                    ((u32::from(straight[at + channel]) * alpha + uncovered + 127) / 255) as u8;
            }
            opaque[at + 3] = 255;
            at += 4;
        }
        opaque
    }

    /// Opaque BGRA from `width × height` to `to_width × to_height` by AREA AVERAGING — each target
    /// pixel the mean of the source area it covers, fractions of a pixel weighed as such — which is
    /// what a downscale of a picture with hard pixel-art edges wants, and the plan never scales up.
    /// The same size comes back as a copy.
    ///
    /// .NET's `MathF.Round` rounds half to even, and so does this (`round_ties_even`).
    pub fn scale(bgra: &[u8], width: i32, height: i32, to_width: i32, to_height: i32) -> Vec<u8> {
        if width == to_width && height == to_height {
            return bgra.to_vec();
        }
        let (width, height) = (width.max(0) as usize, height.max(0) as usize);
        let (to_width, to_height) = (to_width.max(0) as usize, to_height.max(0) as usize);
        let across = weights(width, to_width);
        let down = weights(height, to_height);
        // Across first: every source row to the target's width, three channels as floats.
        let mut rows = vec![0f32; height * to_width * 3];
        for y in 0..height {
            for (x, covering) in across.iter().enumerate() {
                let (mut b, mut g, mut r) = (0f32, 0f32, 0f32);
                for &(from, weight) in covering {
                    let at = (y * width + from) * 4;
                    b += f32::from(bgra[at]) * weight;
                    g += f32::from(bgra[at + 1]) * weight;
                    r += f32::from(bgra[at + 2]) * weight;
                }
                let into = (y * to_width + x) * 3;
                rows[into] = b;
                rows[into + 1] = g;
                rows[into + 2] = r;
            }
        }
        let mut scaled = vec![0u8; to_width * to_height * 4];
        for (y, covering) in down.iter().enumerate() {
            for x in 0..to_width {
                let (mut b, mut g, mut r) = (0f32, 0f32, 0f32);
                for &(from, weight) in covering {
                    let at = (from * to_width + x) * 3;
                    b += rows[at] * weight;
                    g += rows[at + 1] * weight;
                    r += rows[at + 2] * weight;
                }
                let into = (y * to_width + x) * 4;
                scaled[into] = channel(b);
                scaled[into + 1] = channel(g);
                scaled[into + 2] = channel(r);
                scaled[into + 3] = 255;
            }
        }
        scaled
    }

    /// A float channel as a byte: rounded half to even, as `MathF.Round`, and held to 0–255.
    pub(crate) fn channel(value: f32) -> u8 {
        (value.round_ties_even() as i32).clamp(0, 255) as u8
    }

    /// For each of `to` target pixels, the source pixels it covers and how much of it each makes
    /// up.
    pub(crate) fn weights(from: usize, to: usize) -> Vec<Vec<(usize, f32)>> {
        let step = from as f64 / to as f64;
        (0..to)
            .map(|target| {
                let start = target as f64 * step;
                let end = (target + 1) as f64 * step;
                let last = from.min(end.ceil() as usize);
                (start.floor() as usize..last)
                    .filter_map(|source| {
                        let covered = end.min((source + 1) as f64) - start.max(source as f64);
                        (covered > 0.0).then(|| (source, (covered / step) as f32))
                    })
                    .collect()
            })
            .collect()
    }

    /// Opaque BGRA as NV12 — a full-size Y plane, then interleaved Cb Cr at half size each way — in
    /// BT.709 LIMITED range, the profile's "8-bit SDR BT.709". Converted here rather than by the
    /// encoder's own colour conversion, so the matrix is never chosen by resolution. `width` and
    /// `height` are even, as the plan makes them.
    ///
    /// For GStreamer: `video/x-raw,format=NV12,colorimetry=bt709` (range 16–235).
    pub fn nv12(bgra: &[u8], width: i32, height: i32) -> Vec<u8> {
        let (width, height) = (width.max(0) as usize, height.max(0) as usize);
        let luma = width * height;
        let mut nv12 = vec![0u8; luma + luma / 2];
        for (at, value) in nv12.iter_mut().enumerate().take(luma) {
            *value = y(
                i32::from(bgra[at * 4 + 2]),
                i32::from(bgra[at * 4 + 1]),
                i32::from(bgra[at * 4]),
            );
        }
        let mut chroma = luma;
        for top in (0..height).step_by(2) {
            for left in (0..width).step_by(2) {
                let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
                for (dx, dy) in QUAD {
                    let at = ((top + dy) * width + left + dx) * 4;
                    b += i32::from(bgra[at]);
                    g += i32::from(bgra[at + 1]);
                    r += i32::from(bgra[at + 2]);
                }
                let (cb, cr) = cb_cr(r, g, b, 4);
                nv12[chroma] = cb;
                nv12[chroma + 1] = cr;
                chroma += 2;
            }
        }
        nv12
    }

    const QUAD: [(usize, usize); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];

    // BT.709: Kr 0.2126, Kb 0.0722. Limited range: Y 16–235 (219 steps), Cb Cr 16–240 (224) around
    // 128. Fixed point at 2^16, each chroma row's three weights summing to exactly zero so a grey is
    // exactly 128.
    const YR: i64 = 11966; // 0.2126 × 219/255 × 65536
    const YG: i64 = 40254; // 0.7152 × 219/255 × 65536
    const YB: i64 = 4064; // 0.0722 × 219/255 × 65536
    const CB_R: i64 = -6596; // −0.2126/1.8556 × 224/255 × 65536
    const CB_G: i64 = -22189; // −0.7152/1.8556 × 224/255 × 65536
    const CB_B: i64 = 28785; // 0.5 × 224/255 × 65536, less the rounding of the other two
    const CR_R: i64 = 28785;
    const CR_G: i64 = -26145; // −0.7152/1.5748 × 224/255 × 65536
    const CR_B: i64 = -2640; // the rest, so the row sums to zero

    /// One pixel's BT.709 limited-range luma.
    pub fn y(r: i32, g: i32, b: i32) -> u8 {
        let weighted = (YR * i64::from(r) + YG * i64::from(g) + YB * i64::from(b) + 32768) >> 16;
        (16 + weighted).clamp(16, 235) as u8
    }

    /// The BT.709 limited-range chroma of `count` pixels' summed channels.
    pub fn cb_cr(r: i32, g: i32, b: i32, count: i32) -> (u8, u8) {
        let scale = 65536 * i64::from(count);
        let rounded = |value: i64| {
            if value >= 0 {
                (value + scale / 2) / scale
            } else {
                -((-value + scale / 2) / scale)
            }
        };
        let (r, g, b) = (i64::from(r), i64::from(g), i64::from(b));
        let cb = 128 + rounded(CB_R * r + CB_G * g + CB_B * b);
        let cr = 128 + rounded(CR_R * r + CR_G * g + CR_B * b);
        (cb.clamp(16, 240) as u8, cr.clamp(16, 240) as u8)
    }
}

/// Why the next video frame could not be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Source frame `frame` could not be decoded, or decoded at another size than its
    /// `width × height` (the C#'s `InvalidDataException`): the animation ends as a failure
    /// (rule C).
    Undecodable {
        frame: usize,
        width: i32,
        height: i32,
    },
    /// The `Cancel` was raised (the C#'s `OperationCanceledException`).
    Canceled,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Undecodable {
                frame,
                width,
                height,
            } => write!(
                f,
                "frame {frame} of an animation could not be decoded at {width}x{height}"
            ),
            FrameError::Canceled => f.write_str("the operation was cancelled"),
        }
    }
}

impl std::error::Error for FrameError {}

/// The animation's video frames, one after another: each source frame composited in turn
/// ([`FrameCanvas`]), and the canvas taken at every one the plan samples — on white, at the plan's
/// size.
///
/// EVERY FRAME IS DRAWN, SAMPLED OR NOT. A frame no tick lands on is not in the video, but what it
/// left on the canvas is: the next frame is drawn over it. So the source is walked in order, start
/// to end, once — and only one canvas, plus what a "previous" disposal keeps, is ever held,
/// whatever the animation's length.
pub struct AnimationFrames {
    layout: AnimationLayout,
    plan: AnimationPlan,
    pixels_of: PixelsOf,
    canvas: FrameCanvas,
    /// How many source frames are on the canvas.
    drawn: usize,
    next: usize,
}

impl AnimationFrames {
    /// The frames of `layout` as `plan` samples them, each source frame's pixels from `pixels_of`.
    /// `None` when the layout's canvas cannot be made (a side that is not positive, or too large
    /// to count — where the C#'s constructor threw).
    pub fn new(layout: AnimationLayout, plan: AnimationPlan, pixels_of: PixelsOf) -> Option<Self> {
        let canvas = FrameCanvas::new(layout.width, layout.height)?;
        Some(AnimationFrames {
            layout,
            plan,
            pixels_of,
            canvas,
            drawn: 0,
            next: 0,
        })
    }

    /// How many video frames have been given out.
    pub fn given(&self) -> usize {
        self.next
    }

    /// The plan the frames follow.
    pub fn plan(&self) -> &AnimationPlan {
        &self.plan
    }

    /// The next video frame as opaque BGRA at the plan's size, or `Ok(None)` once every sample has
    /// been given.
    pub async fn next(&mut self, cancel: &Cancel) -> Result<Option<Vec<u8>>, FrameError> {
        let Some(sample) = self.plan.samples.get(self.next) else {
            return Ok(None);
        };
        let wanted = usize::try_from(sample.frame).unwrap_or(usize::MAX);
        while self.drawn <= wanted {
            if cancel.is_cancelled() {
                return Err(FrameError::Canceled);
            }
            yield_now().await;
            if cancel.is_cancelled() {
                return Err(FrameError::Canceled);
            }
            let index = self.drawn;
            let Some(frame) = self.layout.frames.get(index).copied() else {
                // A plan that is not this layout's: nothing to draw the sample from.
                return Err(FrameError::Undecodable {
                    frame: index,
                    width: 0,
                    height: 0,
                });
            };
            let undecodable = FrameError::Undecodable {
                frame: index,
                width: frame.width,
                height: frame.height,
            };
            let pixels = (self.pixels_of)(index).await.ok_or(undecodable)?;
            self.canvas
                .draw(&frame, &pixels)
                .map_err(|WrongSize| undecodable)?;
            self.drawn += 1;
        }
        self.next += 1;
        Ok(Some(animation_render::scale(
            &animation_render::on_white(self.canvas.pixels()),
            self.canvas.width(),
            self.canvas.height(),
            self.plan.width,
            self.plan.height,
        )))
    }
}

/// An animated sticker's frames as they are drawn in a chat: every frame of the file composited by
/// this client — the [`FrameCanvas`] #96 composites an animation with — kept TRANSPARENT, and
/// scaled to the size the sticker is drawn at (`StickerFrames`).
///
/// WHY THIS AND NOT THE PLATFORM'S FRAMES. On Windows the platform handed out one frame of an APNG
/// and one (or the raw rectangles) of an animated WebP, so an animated sticker was drawn still.
/// gdk-pixbuf animates GIF and WebP but not APNG; this composites all three the same way.
///
/// PREMULTIPLIED, AND SCALED IN PREMULTIPLIED SPACE. GDK's textures take premultiplied BGRA
/// (`gdk::MemoryFormat::B8g8r8a8Premultiplied`), and averaging straight-alpha pixels lets the
/// colour of a fully transparent pixel bleed into the sticker's edge as a dark or coloured fringe;
/// averaging premultiplied ones cannot.
pub mod sticker_frames {
    use super::*;

    /// Every frame of `layout`, composited in order and drawn at `to_width × to_height` as
    /// premultiplied BGRA — or `Ok(None)` when any frame's pixels could not be had, when a size is
    /// not positive (where the C# threw) or the canvas too large to make; `Err` once `cancel` is
    /// raised.
    pub async fn compose(
        layout: &AnimationLayout,
        pixels_of: &PixelsOf,
        to_width: i32,
        to_height: i32,
        cancel: &Cancel,
    ) -> Result<Option<Vec<Vec<u8>>>, OperationCanceled> {
        if to_width <= 0 || to_height <= 0 {
            return Ok(None);
        }
        let Some(mut canvas) = FrameCanvas::new(layout.width, layout.height) else {
            return Ok(None);
        };
        let mut drawn = Vec::with_capacity(layout.frames.len());
        for (at, frame) in layout.frames.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(OperationCanceled);
            }
            yield_now().await;
            if cancel.is_cancelled() {
                return Err(OperationCanceled);
            }
            let Some(pixels) = pixels_of(at).await else {
                return Ok(None);
            };
            if canvas.draw(frame, &pixels).is_err() {
                return Ok(None);
            }
            drawn.push(premultiplied(
                canvas.pixels(),
                canvas.width(),
                canvas.height(),
                to_width,
                to_height,
            ));
        }
        Ok(Some(drawn))
    }

    /// Straight BGRA from `width × height` to premultiplied BGRA at `to_width × to_height`, by the
    /// same area averaging as [`animation_render::scale`] — alpha included.
    pub fn premultiplied(
        straight: &[u8],
        width: i32,
        height: i32,
        to_width: i32,
        to_height: i32,
    ) -> Vec<u8> {
        let (width, height) = (width.max(0) as usize, height.max(0) as usize);
        let (to_width, to_height) = (to_width.max(0) as usize, to_height.max(0) as usize);
        let mut premultiplied = vec![0f32; width * height * 4];
        let mut at = 0;
        while at + 3 < straight.len() && at + 3 < premultiplied.len() {
            let alpha = f32::from(straight[at + 3]);
            for channel in 0..3 {
                premultiplied[at + channel] = f32::from(straight[at + channel]) * alpha / 255f32;
            }
            premultiplied[at + 3] = alpha;
            at += 4;
        }
        let across = animation_render::weights(width, to_width);
        let down = animation_render::weights(height, to_height);
        let mut rows = vec![0f32; height * to_width * 4];
        for y in 0..height {
            for (x, covering) in across.iter().enumerate() {
                let into = (y * to_width + x) * 4;
                for &(from, weight) in covering {
                    let at = (y * width + from) * 4;
                    for channel in 0..4 {
                        rows[into + channel] += premultiplied[at + channel] * weight;
                    }
                }
            }
        }
        let mut scaled = vec![0u8; to_width * to_height * 4];
        for (y, covering) in down.iter().enumerate() {
            for x in 0..to_width {
                let into = (y * to_width + x) * 4;
                for channel in 0..4 {
                    let mut value = 0f32;
                    for &(from, weight) in covering {
                        value += rows[(from * to_width + x) * 4 + channel] * weight;
                    }
                    scaled[into + channel] = animation_render::channel(value);
                }
                // Rounding can leave a colour a hair above its alpha, which premultiplied BGRA does
                // not allow.
                let alpha = scaled[into + 3];
                for channel in 0..3 {
                    scaled[into + channel] = scaled[into + channel].min(alpha);
                }
            }
        }
        scaled
    }
}

/// Once Pending, then Ready: hands the executor back its turn — on the GLib main context, the
/// window's — between two frames.
pub(crate) fn yield_now() -> impl Future<Output = ()> {
    struct YieldNow(bool);
    impl Future for YieldNow {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                return Poll::Ready(());
            }
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
    YieldNow(false)
}
