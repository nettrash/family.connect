//! An animated picture made into the protocol's silent looping video (issue
//! #96, docs/protocol.md "Animations").
//!
//! The TIMING is not decided here: `fc_text::animation::plan` says which
//! source frame is shown for how many thirtieths of a second, from the
//! delays the picture states (`fc_text::animation_source`), and every port is
//! held to the same answers. This is the part only a browser can do: the
//! frames, COMPOSITED as a viewer shows them — disposal and blending applied
//! — come from WebCodecs' `ImageDecoder`; each is drawn on WHITE at the
//! plan's size, because a video has no transparency; and the browser's
//! H.264 encoder makes the picture track, which the crate's own MP4 writer
//! puts behind its index. No sound track at all.
//!
//! Each video frame lasts exactly its ticks: the track counts time on the
//! plan's own grid ([`TIMESCALE`] = 30 a second), so a sample's duration IS
//! its tick count.
//!
//! As everywhere in crate::encode, None is "this browser could not": no
//! `ImageDecoder` (Safari), a type it does not decode, a frame count that is
//! not the one the picture states, an encoder that refused or failed. The
//! caller then sends the picture exactly as it went before (rule C).

use super::*;

use fc_text::animation::{AnimationPlan, TICKS_PER_SECOND};

use crate::webcodecs::{ImageDecoder, Latency};

/// Ticks a second of an animation's picture track: the plan's own grid.
pub const TIMESCALE: u32 = TICKS_PER_SECOND;

/// What an animation became: the MP4, and its first frame as it was drawn —
/// at the video's size, on white — for the poster to be made from.
pub struct Animated {
    pub blob: Blob,
    pub first: HtmlCanvasElement,
}

/// A decoder, closed when it is let go of: it holds every byte of the
/// picture, and decoded frames besides.
struct Decoder(ImageDecoder);

impl Drop for Decoder {
    fn drop(&mut self) {
        // Closing twice, or after an error closed it, throws.
        let _ = self.0.close();
    }
}

/// A canvas of `width × height` and its 2D context.
fn canvas(width: u32, height: u32) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d)> {
    let document = web_sys::window()?.document()?;
    let canvas: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(width);
    canvas.set_height(height);
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let _ = Reflect::set(
        &context,
        &JsValue::from_str("imageSmoothingQuality"),
        &JsValue::from_str("high"),
    );
    Some((canvas, context))
}

/// Microseconds from the start at `ticks` of [`TIMESCALE`], as a
/// `VideoFrame` counts time — whole, and read back to the very tick.
fn micros_at(ticks: u64) -> f64 {
    (ticks as f64 * 1e6 / f64::from(TIMESCALE)).round()
}

/// `bytes` — a picture of type `mime` whose frames are stated to number
/// `frames` — made into the MP4 `plan` describes: every sample's frame
/// decoded composited, drawn on white at the plan's size, encoded as H.264
/// at the plan's bitrate, each lasting its ticks.
pub async fn animation(
    bytes: &Uint8Array,
    mime: &str,
    frames: usize,
    plan: &AnimationPlan,
    job: &Job,
) -> Option<Animated> {
    if plan.samples.is_empty() || !webcodecs::decodes_image(mime).await {
        return None;
    }
    let (width, height) = (
        u16::try_from(plan.width).ok()?,
        u16::try_from(plan.height).ok()?,
    );
    let encoder_config = webcodecs::h264_config(
        plan.width,
        plan.height,
        f64::from(TICKS_PER_SECOND),
        plan.video_bitrate,
        Latency::Quality,
    )
    .await?;
    let decoder = Decoder(
        ImageDecoder::new(&object(&[
            ("data", bytes.into()),
            ("type", JsValue::from_str(mime)),
            ("preferAnimation", JsValue::TRUE),
        ]))
        .ok()?,
    );
    // The frame count is final once every byte has been read; until then a
    // GIF's is however many frames have turned up so far.
    let tracks = decoder.0.tracks().ok()?;
    let ready = Reflect::get(&tracks, &JsValue::from_str("ready"))
        .ok()?
        .dyn_into::<Promise>()
        .ok()?;
    if !settled(Ok(ready)).await || !settled(decoder.0.completed()).await {
        return None;
    }
    let track = Reflect::get(&tracks, &JsValue::from_str("selectedTrack")).ok()?;
    let count = Reflect::get(&track, &JsValue::from_str("frameCount"))
        .ok()?
        .as_f64()?;
    // The delays were read from the bytes, frame by frame; a decoder that
    // sees another number of frames is not showing the picture they belong
    // to, and nothing here can say which frame is which.
    if count != frames as f64 {
        log::warn!(
            "An animation's decoder saw {count} frames where it states {frames}; sent as it is"
        );
        return None;
    }

    let (easel, context) = canvas(plan.width, plan.height)?;
    let (w, h) = (f64::from(plan.width), f64::from(plan.height));
    let (wake, mut events) = mpsc::unbounded();
    let sink = Rc::new(RefCell::new(PictureSink::default()));
    let encoder = {
        let sink = sink.clone();
        let waker = wake.clone();
        let (init, output, failure) = callbacks(
            move |chunk: JsValue, metadata: JsValue| {
                let chunk: EncodedVideoChunk = chunk.unchecked_into();
                let bytes = Uint8Array::new_with_length(chunk.byte_length());
                let mut sink = sink.borrow_mut();
                if chunk.copy_to(&bytes).is_err() {
                    sink.failed = true;
                }
                sink.chunks
                    .push((bytes, chunk.timestamp(), chunk.kind() == "key"));
                if sink.avcc.is_none() {
                    sink.avcc = decoder_config_field(&metadata, "description")
                        .and_then(|description| bytes_of(&description))
                        .filter(|avcc| !avcc.is_empty());
                }
                if sink.colour.is_none() {
                    sink.colour = decoder_config_field(&metadata, "colorSpace");
                }
                let _ = waker.unbounded_send(Event::Progress);
            },
            &wake,
        );
        Codec {
            codec: VideoEncoder::new(&init).ok()?,
            _output: output,
            _failure: failure,
        }
    };
    encoder.codec.configure(&encoder_config).ok()?;

    let mut first = None;
    let mut tick = 0u64;
    let mut last_key = f64::NEG_INFINITY;
    let mut silence = 0u32;
    for (index, sample) in plan.samples.iter().enumerate() {
        if job.stopped() {
            return None;
        }
        job.progress(index, plan.samples.len());
        while encoder.codec.encode_queue_size() >= QUEUE {
            if let Event::Failed = next(&mut events, &mut silence).await? {
                return None;
            }
        }
        let decoded = JsFuture::from(
            decoder
                .0
                .decode(&object(&[
                    (
                        "frameIndex",
                        JsValue::from(u32::try_from(sample.frame).ok()?),
                    ),
                    ("completeFramesOnly", JsValue::TRUE),
                ]))
                .ok()?,
        )
        .await
        .ok()?;
        let image = Frame(
            Reflect::get(&decoded, &JsValue::from_str("image"))
                .ok()?
                .dyn_into::<Object>()
                .ok()?
                .unchecked_into(),
        );
        // White first, wherever the picture is transparent — then the
        // frame, scaled to the plan's size.
        let [r, g, b] = fc_text::animation::BACKGROUND_RGB;
        context.set_fill_style_str(&format!("rgb({r},{g},{b})"));
        context.fill_rect(0.0, 0.0, w, h);
        let frames: &FrameCanvas = context.unchecked_ref();
        frames.draw_frame(&image.0, 0.0, 0.0, w, h).ok()?;
        drop(image);
        if first.is_none() {
            let (copy, copy_context) = canvas(plan.width, plan.height)?;
            copy_context
                .draw_image_with_html_canvas_element(&easel, 0.0, 0.0)
                .ok()?;
            first = Some(copy);
        }
        let shown = micros_at(tick);
        let ticks = u64::from(sample.ticks);
        let frame = Frame(
            VideoFrame::new_from_image(
                easel.as_ref(),
                &object(&[
                    ("timestamp", JsValue::from(shown)),
                    ("duration", JsValue::from(micros_at(tick + ticks) - shown)),
                    ("alpha", JsValue::from_str("discard")),
                ]),
            )
            .ok()?,
        );
        let key = shown - last_key >= KEYFRAME_SECONDS * 1e6;
        if key {
            last_key = shown;
        }
        encoder
            .codec
            .encode(&frame.0, &object(&[("keyFrame", JsValue::from(key))]))
            .ok()?;
        tick += ticks;
    }
    drop(decoder);
    if !settled(encoder.codec.flush()).await {
        return None;
    }
    while let Ok(event) = events.try_recv() {
        if matches!(event, Event::Failed) {
            return None;
        }
    }
    drop(encoder);
    let sink = std::mem::take(&mut *sink.borrow_mut());
    let kept = plan.samples.len();
    if sink.failed || sink.chunks.len() != kept || !sink.chunks[0].2 {
        log::warn!(
            "An animation's encoder made {} frames of {kept}; sent as it is",
            sink.chunks.len()
        );
        return None;
    }
    let avcc = sink.avcc?;
    let colour = match sink.colour.as_ref().map(colour) {
        Some(Some(colour)) if matches!(colour.transfer, 16 | 18) => return None,
        Some(colour) => colour,
        None => None,
    };
    // Back on the grid: each frame's time, to the very tick it was given.
    let pts: Vec<i64> = sink
        .chunks
        .iter()
        .map(|(_, shown, _)| (shown * f64::from(TIMESCALE) / 1e6).round() as i64)
        .collect();
    let last = plan.samples.last()?.ticks;
    let samples: Vec<mp4::Sample> = mp4::decode_timeline(&pts, last)
        .into_iter()
        .zip(&sink.chunks)
        .map(
            |((duration, composition_offset), (bytes, _, key))| mp4::Sample {
                size: bytes.length(),
                duration,
                composition_offset,
                sync: *key,
            },
        )
        .collect();
    let picture = mp4::Track {
        timescale: TIMESCALE,
        media: Media::Video {
            width,
            height,
            avcc,
            colour,
        },
        samples,
        skip: 0,
        lead: 0,
    };
    let layout = mp4::layout(std::slice::from_ref(&picture), Brand::Mp4);
    let parts: Vec<JsValue> = sink
        .chunks
        .into_iter()
        .map(|(bytes, _, _)| bytes.into())
        .collect();
    let blob = assemble(
        &layout.header,
        layout
            .order
            .iter()
            .map(|&(_, sample)| parts[sample].clone()),
        "video/mp4",
    )?;
    (blob.size() as u64 == layout.total_len).then_some(Animated {
        blob,
        first: first?,
    })
}

/// Pictures that move, for the tests of every module that prepares or draws
/// one.
#[cfg(test)]
pub mod testing {
    /// LZW for a GIF of four colours (a code size of 2), as simply as it can
    /// be written and still be read by every decoder: each pixel its own
    /// code, and the table cleared after every two, so the codes never grow
    /// past three bits.
    pub fn lzw(pixels: &[u8]) -> Vec<u8> {
        let (clear, end) = (4u32, 5u32);
        let mut codes = Vec::new();
        for pair in pixels.chunks(2) {
            codes.push(clear);
            codes.extend(pair.iter().map(|&pixel| u32::from(pixel)));
        }
        codes.push(end);
        let (mut bytes, mut bits, mut held) = (Vec::new(), 0u32, 0u32);
        for code in codes {
            held |= code << bits;
            bits += 3;
            while bits >= 8 {
                bytes.push((held & 0xFF) as u8);
                held >>= 8;
                bits -= 8;
            }
        }
        if bits > 0 {
            bytes.push(held as u8);
        }
        bytes
    }

    /// A real animated GIF of `width × height`: frame `k` is colour `k` of
    /// red, green and blue, with its top-left quarter TRANSPARENT (index 3)
    /// and disposed to the background — which a viewer shows as nothing —
    /// each stated at `delays[k]` hundredths of a second.
    pub fn animated_gif(width: u16, height: u16, delays: &[u16]) -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&width.to_le_bytes());
        gif.extend_from_slice(&height.to_le_bytes());
        // A global table of four colours, background index 3.
        gif.extend_from_slice(&[0x81, 3, 0]);
        gif.extend_from_slice(&[0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0x80, 0x80, 0x80]);
        gif.extend_from_slice(&[0x21, 0xFF, 11]);
        gif.extend_from_slice(b"NETSCAPE2.0");
        gif.extend_from_slice(&[3, 1, 0, 0, 0]);
        for (index, delay) in delays.iter().enumerate() {
            let [lo, hi] = delay.to_le_bytes();
            // Disposal 2 (to the background), transparency on index 3.
            gif.extend_from_slice(&[0x21, 0xF9, 4, 0x09, lo, hi, 3, 0]);
            gif.push(0x2C);
            gif.extend_from_slice(&[0, 0, 0, 0]);
            gif.extend_from_slice(&width.to_le_bytes());
            gif.extend_from_slice(&height.to_le_bytes());
            gif.push(0);
            let colour = (index % 3) as u8;
            let pixels: Vec<u8> = (0..height)
                .flat_map(|y| {
                    (0..width).map(move |x| {
                        if x < width / 2 && y < height / 2 {
                            3
                        } else {
                            colour
                        }
                    })
                })
                .collect();
            gif.push(2);
            for block in lzw(&pixels).chunks(255) {
                gif.push(block.len() as u8);
                gif.extend_from_slice(block);
            }
            gif.push(0);
        }
        gif.push(0x3B);
        gif
    }

    /// The CRC every PNG chunk ends with (ISO 3309, as the PNG spec gives it).
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut typed = kind.to_vec();
        typed.extend_from_slice(data);
        out.extend_from_slice(&typed);
        out.extend_from_slice(&crc32(&typed).to_be_bytes());
        out
    }

    /// `raw` as a zlib stream of stored (uncompressed) blocks.
    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (index, block) in blocks.iter().enumerate() {
            out.push(u8::from(index + 1 == blocks.len()));
            let length = block.len() as u16;
            out.extend_from_slice(&length.to_le_bytes());
            out.extend_from_slice(&(!length).to_le_bytes());
            out.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in raw {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    /// A real animated PNG of `width × height`, RGBA: frame `k` is red,
    /// green or blue with its top-left quarter TRANSPARENT, each stated at
    /// `delays[k]` hundredths of a second (`delay_den` 100); the default
    /// image is the first frame.
    pub fn animated_png(width: u32, height: u32, delays: &[u16]) -> Vec<u8> {
        let image = |index: usize| {
            let colour = match index % 3 {
                0 => [0xFF, 0, 0, 0xFF],
                1 => [0, 0xFF, 0, 0xFF],
                _ => [0, 0, 0xFF, 0xFF],
            };
            let mut raw = Vec::new();
            for y in 0..height {
                raw.push(0);
                for x in 0..width {
                    if x < width / 2 && y < height / 2 {
                        raw.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        raw.extend_from_slice(&colour);
                    }
                }
            }
            zlib_stored(&raw)
        };
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend(chunk(b"IHDR", &ihdr));
        let mut actl = (delays.len() as u32).to_be_bytes().to_vec();
        actl.extend_from_slice(&0u32.to_be_bytes());
        png.extend(chunk(b"acTL", &actl));
        let mut sequence = 0u32;
        for (index, delay) in delays.iter().enumerate() {
            let mut fctl = sequence.to_be_bytes().to_vec();
            sequence += 1;
            fctl.extend_from_slice(&width.to_be_bytes());
            fctl.extend_from_slice(&height.to_be_bytes());
            fctl.extend_from_slice(&[0; 8]);
            fctl.extend_from_slice(&delay.to_be_bytes());
            fctl.extend_from_slice(&100u16.to_be_bytes());
            // Dispose to nothing, and drawn over nothing: each frame whole.
            fctl.extend_from_slice(&[1, 0]);
            png.extend(chunk(b"fcTL", &fctl));
            if index == 0 {
                png.extend(chunk(b"IDAT", &image(index)));
            } else {
                let mut fdat = sequence.to_be_bytes().to_vec();
                sequence += 1;
                fdat.extend(image(index));
                png.extend(chunk(b"fdAT", &fdat));
            }
        }
        png.extend(chunk(b"IEND", &[]));
        png
    }
}
