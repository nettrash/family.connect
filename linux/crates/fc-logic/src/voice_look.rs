//! How a VOICE MESSAGE is drawn on this client — `VoiceLook.cs`, the approved design of 2026-10-05
//! (the "Voice and Video Messages" mockup): a round accent play button, the waveform the sender
//! measured (docs/protocol.md, "A voice note's waveform"), the played bars in the accent as it
//! plays, the time in tabular digits, a speed chip while it plays, and a dot until this device has
//! played someone else's. A drawing rule, not a wire one: the sizes, which bars, what is written
//! and what a screen reader hears. Beside it: the speed ([`voice_speed`]), the recording row's live
//! line ([`LiveLevels`], fed by the shell's [`VoiceMeter`]), and the waveform a recording made here
//! is sent with ([`voice_waveform`], measured through the shell's [`VoiceDecoder`]).
//!
//! **THE WAVEFORM IS THE SENDER'S, OR A NEUTRAL ONE.** 48 levels from the attachment, reduced to the
//! bars that fit by the shared rule (`fc_text::waveform::bars`); with none, or one that does not
//! parse, every bar is the placeholder's level — never a shape invented here, which would pass for
//! the recording's own.
//!
//! **PLAYED BARS ARE ⌊position · bars / duration⌋**, the shared rule
//! (`fc_text::waveform::played_bars`), so a note half played shows half its bars lit on every
//! client.

use std::collections::VecDeque;

use fc_core::protocol::AttachmentDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::media::time_label;
use fc_text::record::MIN_TARGET_WINDOWS_EPX;
use fc_text::waveform;

use crate::voice_notes;

/// The bubble's width, in logical pixels: the mockup's `min(300, 100%)` at the width that fits
/// every surface this client draws one on — the thread panel leaves a reply 295 beside its 72-wide
/// indent, so 280, never clipped.
pub const WIDTH: f64 = 280.0;

/// The round play button's disc (the mockup's 40), in a target of
/// `fc_text::record::MIN_TARGET_WINDOWS_EPX` (44, the desktop's target).
pub const PLAY_DISC: f64 = 40.0;

/// The waveform's height in the bubble.
pub const WAVE_HEIGHT: f64 = 28.0;

/// The mini waveform's height in a review or not-sent chip.
pub const CHIP_WAVE_HEIGHT: f64 = 22.0;

/// A bar's width.
pub const BAR_WIDTH: f64 = 3.0;

/// The gap between two bars.
pub const BAR_GAP: f64 = 2.0;

/// The unplayed dot.
pub const DOT: f64 = 7.0;

/// The waveform's width in the bubble: what is left of [`WIDTH`] after its padding (8 and 12), the
/// play disc and the gap beside it (10).
pub const WAVE_WIDTH: f64 = WIDTH - 8.0 - 12.0 - PLAY_DISC - 10.0;

/// The not-sent chip at its widest: the mockup's `min(360px, 100%)`.
pub const CHIP_WIDTH: f64 = 360.0;

/// The fewest bars the not-sent chip's mini waveform keeps on its line before "Not sent" moves
/// above it.
pub const MIN_CHIP_BARS: usize = 8;

/// How many bars of [`BAR_WIDTH`] with [`BAR_GAP`] between fit in `width`.
pub fn bar_count(width: f64) -> usize {
    if width.is_finite() && width >= BAR_WIDTH {
        ((width + BAR_GAP) / (BAR_WIDTH + BAR_GAP)).floor() as usize
    } else {
        0
    }
}

/// The bars to draw: the attachment's waveform — or the placeholder — reduced to `count`.
pub fn bars(waveform: Option<&str>, count: usize) -> Vec<u8> {
    waveform::bars(&waveform::levels_or_placeholder(waveform), count)
}

/// One bar's height in a waveform `height` tall: `(2 + level) / 17` of it.
pub fn bar_height(level: u8, height: f64) -> f64 {
    waveform::bar_fraction(level) * height.max(0.0)
}

/// How many bars are lit at `position_seconds` of `total_seconds` — the shared rule on whole
/// milliseconds (half rounded away from zero); none for a length nobody knows.
pub fn played_bars(position_seconds: f64, total_seconds: f64, bars: usize) -> usize {
    if !position_seconds.is_finite() || !total_seconds.is_finite() || total_seconds <= 0.0 {
        return 0;
    }
    // `Math.Round(…, MidpointRounding.AwayFromZero)` is Rust's `round`; the casts saturate.
    let at = (position_seconds.max(0.0) * 1000.0).round() as u64;
    let total = (total_seconds * 1000.0).round() as u64;
    waveform::played_bars(at, total, bars)
}

/// The time under the waveform: where it is while it is the one playing (or paused part way), its
/// whole length at rest — the mockup's "0:42" that counts up as it plays.
pub fn time(active: bool, position_seconds: f64, total_seconds: f64) -> String {
    if active {
        // `Math.Clamp(position, 0, Math.Max(0, total))` with .NET's NaN rules: a NaN total is no
        // bound (.NET's `Max` keeps the NaN and its `Clamp` then returns the position), and a NaN
        // position reads "0:00" either way. Rust's `clamp` would panic on the NaN bound.
        let most = if total_seconds.is_nan() {
            f64::NAN
        } else {
            total_seconds.max(0.0)
        };
        let at = if position_seconds > most {
            most
        } else {
            position_seconds.max(0.0)
        };
        time_label(at)
    } else {
        time_label(total_seconds)
    }
}

/// The dot: only on SOMEONE ELSE'S voice message that this device has not played — never on the
/// reader's own.
pub fn shows_dot(mine: bool, played: bool) -> bool {
    !mine && !played
}

/// What a screen reader hears for the bubble: "Voice message, 0:42".
pub fn name(audio: &AttachmentDto, say: &dyn StringCatalog) -> String {
    let length = time_label(voice_notes::total_seconds(audio.duration_ms));
    say.format("Voice message, %@", &[&length])
}

/// Its status: "Not played" or "Played" on someone else's, and nothing on the reader's own.
pub fn status(mine: bool, played: bool, say: &dyn StringCatalog) -> Option<String> {
    if mine {
        None
    } else if played {
        Some(say.get("Played").to_string())
    } else {
        Some(say.get("Not played").to_string())
    }
}

/// The speed chip shows while this bubble is the one playing — or paused part way through.
pub fn shows_speed(active: bool) -> bool {
    active
}

/// How far a control's target reaches past what is drawn of it, on each side: the negative margin
/// that makes a small visual — the speed chip, a chip's ✕, its Send — a 44-pixel target (S1.1,
/// `MIN_TARGET_WINDOWS_EPX`) without making the row it sits in any taller. Nothing for a visual
/// that is already that large.
pub fn reach(visual: f64) -> f64 {
    if visual.is_finite() {
        -(f64::from(MIN_TARGET_WINDOWS_EPX) - visual.max(0.0)).max(0.0) / 2.0
    } else {
        0.0
    }
}

/// Where Play starts a bubble that is not playing: where its waveform was SEEKED to while it was
/// idle, or the start — and the start, too, for a seek to the very end, where there would be
/// nothing left to hear.
pub fn start_at(seeked_seconds: Option<f64>, total_seconds: f64) -> f64 {
    match seeked_seconds {
        Some(at)
            if at.is_finite()
                && at > 0.0
                && total_seconds.is_finite()
                && total_seconds > 0.0
                && !voice_notes::replays_from_start(at, total_seconds) =>
        {
            at
        }
        _ => 0.0,
    }
}

/// Whether a send still on its way is ONE VOICE NOTE: no words, and exactly one staged item, a sound
/// — drawn as the bubble it will be (dimmed, with "Sending…") rather than as a count of files.
pub fn is_pending_voice<S: AsRef<str>>(body: Option<&str>, staged_kinds: &[S]) -> bool {
    body.is_none_or(str::is_empty) && matches!(staged_kinds, [only] if only.as_ref() == "audio")
}

/// How the not-sent chip is laid out at its width: whether "Not sent" is on a line of its own, and
/// how many bars it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChipFit {
    pub label_above: bool,
    pub bars: usize,
}

/// The not-sent chip fitted to its width (the mockup's "Not sent", ▶, mini waveform, length, Send
/// and ✕ on one line): its waveform gets the room the other columns leave and is REDUCED to the
/// bars that fit — never more than `design_bars`, and never cut off part way through the shape —
/// and when a language's "Not sent" or a large text size leaves fewer than [`MIN_CHIP_BARS`], "Not
/// sent" moves onto its own line above the rest. `row_width` is the chip's inner width,
/// `others_width` what ▶, the length, Send and ✕ take together, `label_width` what "Not sent"
/// takes, `spacing` the gap between two columns.
pub fn fit_not_sent(
    row_width: f64,
    others_width: f64,
    label_width: f64,
    spacing: f64,
    design_bars: usize,
) -> ChipFit {
    // On one line: six columns, five gaps.
    let inline = design_bars.min(bar_count(
        row_width - others_width - label_width - 5.0 * spacing,
    ));
    if inline >= MIN_CHIP_BARS.min(design_bars) {
        return ChipFit {
            label_above: false,
            bars: inline,
        };
    }
    // "Not sent" above: five columns on the line, four gaps.
    ChipFit {
        label_above: true,
        bars: design_bars.min(bar_count(row_width - others_width - 4.0 * spacing)),
    }
}

/// A voice message's playback speed: 1×, 1.5× and 2×, a press on the chip going round them,
/// REMEMBERED ON THIS DEVICE and applied through the player's own rate — the pitch is the player's
/// business.
///
/// On Linux the shell keeps [`voice_speed::store`]'s text in its settings file (shell-design §10.3:
/// `settings.conf`) and reads it back through [`voice_speed::parse`] — an unreadable or missing value is 1×,
/// never an error (Windows' `VoiceSpeedSetting`, which logged only the error's type). It applies the
/// rate with `gst_play::Play::set_rate`, the pipeline's `audio-filter` a `scaletempo` so a faster
/// voice keeps its pitch (§10.1).
pub mod voice_speed {
    use fc_core::text::string_catalog::StringCatalog;

    /// The speeds, in the order a press goes round them.
    pub const RATES: [f64; 3] = [1.0, 1.5, 2.0];

    /// The speed after `rate`: 1× → 1.5× → 2× → 1×; anything else starts again at 1×.
    pub fn next(rate: f64) -> f64 {
        if rate == 1.0 {
            1.5
        } else if rate == 1.5 {
            2.0
        } else {
            1.0
        }
    }

    /// "1×", "1.5×" or "2×" — catalogue keys, since a decimal separator is the language's.
    pub fn label(rate: f64, say: &dyn StringCatalog) -> String {
        let normal = normal(rate);
        if normal == 1.5 {
            say.get("1.5×").to_string()
        } else if normal == 2.0 {
            say.get("2×").to_string()
        } else {
            say.get("1×").to_string()
        }
    }

    /// What a screen reader hears for the chip: "Playback speed, 1.5×".
    pub fn name(rate: f64, say: &dyn StringCatalog) -> String {
        let label = label(rate, say);
        say.format("Playback speed, %@", &[&label])
    }

    /// One of [`RATES`]: anything else — an unreadable file, a hand-edited one — is 1×.
    pub fn normal(rate: f64) -> f64 {
        if RATES.contains(&rate) { rate } else { 1.0 }
    }

    /// The device's stored choice, read: `"1.5"`, `"2"`, and 1× for anything else or nothing. The
    /// number is read in the invariant form (a point, never the language's comma), around
    /// whitespace.
    pub fn parse(stored: Option<&str>) -> f64 {
        stored
            .and_then(|text| text.trim().parse::<f64>().ok())
            .map_or(1.0, normal)
    }

    /// The choice as it is stored: the invariant number, never the language's — "1", "1.5", "2".
    pub fn store(rate: f64) -> String {
        // Rust's `Display` of these three is .NET's `ToString(InvariantCulture)`: no exponent, no
        // trailing ".0".
        normal(rate).to_string()
    }
}

/// The recording row's LIVE waveform (the approved design: "the live waveform scrolls in from the
/// right"): the newest peaks, one bar per meter tick, the oldest falling off the left.
#[derive(Debug, Clone)]
pub struct LiveLevels {
    levels: VecDeque<u8>,
    capacity: usize,
    heard: usize,
}

impl LiveLevels {
    /// A line keeping `capacity` bars (at least one).
    pub fn new(capacity: usize) -> Self {
        LiveLevels {
            levels: VecDeque::new(),
            capacity: capacity.max(1),
            heard: 0,
        }
    }

    /// How many bars it keeps.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many ticks have been heard since the recording began.
    pub fn heard(&self) -> usize {
        self.heard
    }

    /// One tick's peak, in dBFS — as a level by the shared rule (`fc_text::waveform::level`).
    pub fn push(&mut self, peak_dbfs: f64) {
        self.levels.push_back(waveform::level(peak_dbfs));
        self.heard += 1;
        while self.levels.len() > self.capacity {
            self.levels.pop_front();
        }
    }

    /// A new recording: nothing heard yet.
    pub fn clear(&mut self) {
        self.levels.clear();
        self.heard = 0;
    }

    /// The last `count` levels, oldest first and RIGHT-aligned — the newest is the last bar — with
    /// silence (level 0) before the first one heard, so the line grows in from the right.
    pub fn bars(&self, count: usize) -> Vec<u8> {
        let mut bars = vec![0u8; count];
        let take = self.levels.len().min(count);
        for (slot, level) in bars[count - take..]
            .iter_mut()
            .zip(self.levels.iter().skip(self.levels.len() - take))
        {
            *slot = *level;
        }
        bars
    }
}

/// The recording row's level, LIVE — the shell's meter (Windows' `VoiceMeter`, which had to open the
/// microphone a second time because `MediaCapture` exposes no level).
///
/// On Linux it is the `level` branch of the SAME recording pipeline (shell-design §10.2: `tee` →
/// `queue ! level interval=50000000 post-messages=true ! fakesink`), so the microphone is opened
/// once. Contract: the shell reads each `level` element message on the bus (main context), takes
/// the LOUDEST channel's `peak` value (dB relative to full scale, ≤ 0; digital silence comes as a
/// very low or `-inf` value, which [`LiveLevels::push`] reads as level 0), and keeps the loudest
/// since the last take. [`take_peak_dbfs`](Self::take_peak_dbfs) answers it and forgets it — `None`
/// when no message has come since the last take, so the row adds no bar rather than a silent one.
/// The row takes a peak on its own tick (Windows: every 125 ms, with the clock) and
/// [`LiveLevels::push`]es it. Best effort: no meter (an element missing) is a row with the dot and
/// the clock and no line; it never decides whether a recording starts, runs or is kept. What it
/// hears is drawn and forgotten — written nowhere; the note's own waveform is measured from the
/// recording ([`voice_waveform`]), never from this. Let go with the recording: it is a supertrait of
/// [`crate::voice_notes::VoiceRecording`], so the shell implements it on the recording itself.
pub trait VoiceMeter {
    fn take_peak_dbfs(&self) -> Option<f64>;
}

/// A recording's sound, decoded to 16-bit PCM by the shell for [`voice_waveform::shaped`]:
/// `samples` interleaved, `channels` of them per frame, `rate` frames a second.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

/// Decodes a voice note the shell recorded into the [`Pcm`] its waveform is measured from (Windows'
/// `VoiceShape`, which transcoded the M4A to a 16 kHz mono WAV through Media Foundation).
///
/// Contract: [`decode`](Self::decode) is handed the note's M4A bytes (AAC-LC in MP4, as recorded)
/// and decodes ALL of it — `appsrc` (or a temporary file under `$XDG_CACHE_HOME/family-connect/tmp`,
/// removed before the future ends) `! decodebin3 ! audioconvert ! audioresample !
/// audio/x-raw,format=S16LE,channels=1,rate=16000 ! appsink` (shell-design §10.2) — answering every
/// sample in order: mono 16 kHz is what is asked, but any rate and channel count is read correctly.
/// `None` for anything that goes wrong (a decoder missing, an error on the bus, nothing decoded): the
/// note then goes without a waveform, which the protocol allows — never an error that stops it.
/// Runs off the main context's critical path (GStreamer's own threads; the future awaits the bus EOS
/// from the main context). Dropping the future cancels: the pipeline goes to `Null` and any
/// temporary is deleted. Nothing but an error's domain and code may be logged — the bytes are
/// somebody's voice.
pub trait VoiceDecoder {
    fn decode(&self, m4a: std::rc::Rc<[u8]>) -> fc_core::rt::LocalBoxFuture<'static, Option<Pcm>>;
}

/// A voice note's waveform measured from the RECORDING ITSELF (Windows' `VoiceWaveform` and
/// `VoiceShape`): the note is decoded to PCM on the shell's side ([`VoiceDecoder`]) and its peak per
/// [`voice_waveform::BLOCK_MS`] is what another client's meter would have read every tick. Then the shared rule
/// (`fc_text::waveform::from_peaks`) — the same 48 digits whatever measured them, and the same bytes
/// always give the same digits, so a note sent at once, one sent from review and one sent from its
/// not-sent row all go up with the same shape.
///
/// A WAV this does not understand — not RIFF/WAVE, not 16-bit integer or 32-bit float PCM, no
/// samples — is no waveform at all: the note goes without one and every reader draws the
/// placeholder, which the protocol allows.
pub mod voice_waveform {
    use std::rc::Rc;
    use std::time::Duration;

    use fc_core::rt::Clock;
    use fc_text::waveform;
    use futures_util::future::{Either, select};

    use super::{Pcm, VoiceDecoder};
    use crate::media_outbox::StagedMedia;

    /// One peak per tenth of a second: twice the other clients' 200 ms meter tick, never coarser.
    pub const BLOCK_MS: u32 = 100;

    /// How long a measure may take before the note goes without one: a five-minute note decodes in
    /// well under it.
    pub const CEILING: Duration = Duration::from_secs(15);

    /// A sample's magnitude, 0 to 1, in dBFS; silence is −∞, which the shared rule reads as level 0.
    pub fn dbfs(magnitude: f64) -> f64 {
        if magnitude.is_finite() && magnitude > 0.0 {
            20.0 * magnitude.min(1.0).log10()
        } else {
            f64::NEG_INFINITY
        }
    }

    /// The wire's 48 digits for a WAV, or `None` when it holds nothing this can read.
    pub fn from_wav(wav: &[u8]) -> Option<String> {
        peaks(wav)
            .filter(|peaks| !peaks.is_empty())
            .map(|peaks| waveform::from_peaks(&peaks, waveform::LEVELS))
    }

    /// The wire's 48 digits for decoded PCM, or `None` when it holds no sample.
    pub fn from_pcm(pcm: &Pcm) -> Option<String> {
        pcm_peaks(pcm)
            .filter(|peaks| !peaks.is_empty())
            .map(|peaks| waveform::from_peaks(&peaks, waveform::LEVELS))
    }

    /// The peak of every [`BLOCK_MS`] of decoded PCM, in dBFS and in time order; `None` with no rate,
    /// no channel or no whole frame.
    pub fn pcm_peaks(pcm: &Pcm) -> Option<Vec<f64>> {
        let channels = usize::from(pcm.channels);
        if channels == 0 || pcm.rate == 0 {
            return None;
        }
        let frames = pcm.samples.len() / channels;
        block_peaks(frames, channels, pcm.rate, |frame, channel| {
            (f64::from(pcm.samples[frame * channels + channel])).abs() / 32768.0
        })
    }

    /// The peak of every [`BLOCK_MS`] of a WAV, in dBFS and in time order; `None` when it cannot be
    /// read.
    pub fn peaks(wav: &[u8]) -> Option<Vec<f64>> {
        if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
            return None;
        }
        let mut format: Option<u16> = None;
        let mut channels: Option<u16> = None;
        let mut rate: Option<u32> = None;
        let mut bits: Option<u16> = None;
        let mut data: &[u8] = &[];
        let mut found = false;
        let mut at = 12usize;
        while at + 8 <= wav.len() {
            let id = &wav[at..at + 4];
            let size = u32_le(&wav[at + 4..at + 8]);
            let body = at + 8;
            // A chunk that claims more than is there is read to the end — an encoder streaming a
            // WAV leaves a data chunk's size as it found it.
            let length = usize::try_from(size)
                .unwrap_or(usize::MAX)
                .min(wav.len() - body);
            if id == b"fmt " && length >= 16 {
                format = Some(u16_le(&wav[body..body + 2]));
                channels = Some(u16_le(&wav[body + 2..body + 4]));
                rate = Some(u32_le(&wav[body + 4..body + 8]));
                bits = Some(u16_le(&wav[body + 14..body + 16]));
                if format == Some(0xFFFE) && length >= 26 {
                    // WAVE_FORMAT_EXTENSIBLE: the real format is the sub-format GUID's first two
                    // bytes.
                    format = Some(u16_le(&wav[body + 24..body + 26]));
                }
            } else if id == b"data" {
                data = &wav[body..body + length];
                found = true;
                break;
            }
            at = body + length + (length & 1);
        }
        let (Some(kind), Some(channels), Some(rate), Some(bits)) = (format, channels, rate, bits)
        else {
            return None;
        };
        // The C# reads the rate as a signed int: a rate over 2³¹ − 1 is not one.
        if !found
            || channels == 0
            || rate == 0
            || rate > i32::MAX as u32
            || !((kind == 1 && bits == 16) || (kind == 3 && bits == 32))
        {
            return None;
        }
        let channels = usize::from(channels);
        let width = usize::from(bits / 8) * channels;
        let frames = data.len() / width;
        block_peaks(frames, channels, rate, |frame, channel| {
            let offset = frame * width;
            if kind == 1 {
                let at = offset + channel * 2;
                f64::from(i16::from_le_bytes([data[at], data[at + 1]])).abs() / 32768.0
            } else {
                let at = offset + channel * 4;
                f64::from(f32::from_le_bytes([
                    data[at],
                    data[at + 1],
                    data[at + 2],
                    data[at + 3],
                ]))
                .abs()
            }
        })
    }

    /// The loudest sample of every block of `rate × BLOCK_MS / 1000` frames (at least one), and of
    /// the shorter tail; `None` for no frame.
    fn block_peaks(
        frames: usize,
        channels: usize,
        rate: u32,
        sample: impl Fn(usize, usize) -> f64,
    ) -> Option<Vec<f64>> {
        if frames == 0 {
            return None;
        }
        let per_block = usize::try_from(u64::from(rate) * u64::from(BLOCK_MS) / 1000)
            .unwrap_or(usize::MAX)
            .max(1);
        let mut peaks = Vec::with_capacity(frames.div_ceil(per_block));
        let mut loudest = 0.0f64;
        let mut in_block = 0usize;
        for frame in 0..frames {
            for channel in 0..channels {
                let value = sample(frame, channel);
                if value > loudest {
                    loudest = value;
                }
            }
            in_block += 1;
            if in_block == per_block {
                peaks.push(dbfs(loudest));
                loudest = 0.0;
                in_block = 0;
            }
        }
        if in_block > 0 {
            peaks.push(dbfs(loudest));
        }
        Some(peaks)
    }

    /// The 48 digits for a recording's M4A bytes, or `None` — never an error: the shell decodes it
    /// ([`VoiceDecoder`]) within [`CEILING`] (Windows' `VoiceShape.MeasureAsync`); a decode that
    /// takes longer is dropped, which cancels it.
    pub async fn measure(
        m4a: Rc<[u8]>,
        decoder: &dyn VoiceDecoder,
        clock: &dyn Clock,
    ) -> Option<String> {
        if m4a.is_empty() {
            return None;
        }
        let decoding = decoder.decode(m4a);
        let ceiling = clock.sleep(CEILING);
        match select(decoding, ceiling).await {
            Either::Left((Some(pcm), _)) => from_pcm(&pcm),
            Either::Left((None, _)) | Either::Right(_) => None,
        }
    }

    /// The note with its shape when one could be measured, and as it was otherwise (Windows'
    /// `VoiceShape.ShapedAsync`): only a sound that has none yet is measured.
    pub async fn shaped(
        note: StagedMedia,
        decoder: &dyn VoiceDecoder,
        clock: &dyn Clock,
    ) -> StagedMedia {
        if note.kind != "audio" || note.waveform.is_some() {
            return note;
        }
        match measure(Rc::from(note.bytes.as_slice()), decoder, clock).await {
            Some(waveform) => StagedMedia {
                waveform: Some(waveform),
                ..note
            },
            None => note,
        }
    }

    fn u16_le(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[0], bytes[1]])
    }

    fn u32_le(bytes: &[u8]) -> u32 {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}
