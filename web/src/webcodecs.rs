//! WebCodecs — the browser's own encoders and decoders — bound by hand, and
//! asked at run time what they can do (docs/protocol.md, "Preparing media
//! before upload").
//!
//! By hand, because web-sys only generates these types behind the
//! `web_sys_unstable_apis` cfg, which every build of this crate (trunk,
//! clippy, the browser tests, CI) would then have to be told about. Only the
//! handful of calls this client makes are bound, and every one that can
//! throw returns a `Result`.
//!
//! Support is never assumed from a browser's name or version. Safari, Chrome
//! and Firefox each ship WebCodecs with different codecs behind it, and the
//! same Chrome encodes AAC on macOS and Windows and not on Linux. So each
//! question is put to the browser as `isConfigSupported` with the exact
//! configuration about to be used, and anything but a plain "yes" — the
//! class missing, the call throwing, the promise rejecting — is "no". A "no"
//! is never an error: it sends the file the way it went before (rule C).

use js_sys::{Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use fc_text::transcode::{h264_codec, H264Profile};

#[wasm_bindgen]
extern "C" {
    pub type AudioEncoder;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<AudioEncoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn configure(this: &AudioEncoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn encode(this: &AudioEncoder, data: &AudioData) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn flush(this: &AudioEncoder) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn close(this: &AudioEncoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = encodeQueueSize)]
    pub fn encode_queue_size(this: &AudioEncoder) -> u32;

    pub type AudioDecoder;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<AudioDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn configure(this: &AudioDecoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn decode(this: &AudioDecoder, chunk: &EncodedAudioChunk) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn flush(this: &AudioDecoder) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn close(this: &AudioDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    pub fn decode_queue_size(this: &AudioDecoder) -> u32;

    pub type VideoEncoder;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<VideoEncoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn configure(this: &VideoEncoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn encode(this: &VideoEncoder, frame: &VideoFrame, options: &Object)
        -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn flush(this: &VideoEncoder) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn close(this: &VideoEncoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = encodeQueueSize)]
    pub fn encode_queue_size(this: &VideoEncoder) -> u32;

    pub type VideoDecoder;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<VideoDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn configure(this: &VideoDecoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn decode(this: &VideoDecoder, chunk: &EncodedVideoChunk) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn flush(this: &VideoDecoder) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    pub fn close(this: &VideoDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    pub fn decode_queue_size(this: &VideoDecoder) -> u32;

    pub type AudioData;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<AudioData, JsValue>;
    #[wasm_bindgen(method)]
    pub fn close(this: &AudioData);
    #[wasm_bindgen(method, getter, js_name = numberOfFrames)]
    pub fn number_of_frames(this: &AudioData) -> u32;
    #[wasm_bindgen(method, getter, js_name = numberOfChannels)]
    pub fn number_of_channels(this: &AudioData) -> u32;
    #[wasm_bindgen(method, getter, js_name = sampleRate)]
    pub fn sample_rate(this: &AudioData) -> f64;
    #[wasm_bindgen(method, getter)]
    pub fn timestamp(this: &AudioData) -> f64;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    pub fn copy_to(
        this: &AudioData,
        destination: &js_sys::Float32Array,
        options: &Object,
    ) -> Result<(), JsValue>;

    pub type VideoFrame;
    /// `new VideoFrame(image, init)` — a frame from a canvas.
    #[wasm_bindgen(constructor, catch)]
    pub fn new_from_image(image: &JsValue, init: &Object) -> Result<VideoFrame, JsValue>;
    #[wasm_bindgen(method)]
    pub fn close(this: &VideoFrame);
    #[wasm_bindgen(method, getter)]
    pub fn timestamp(this: &VideoFrame) -> f64;

    pub type EncodedAudioChunk;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<EncodedAudioChunk, JsValue>;
    #[wasm_bindgen(method, getter, js_name = byteLength)]
    pub fn byte_length(this: &EncodedAudioChunk) -> u32;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    pub fn copy_to(this: &EncodedAudioChunk, destination: &Uint8Array) -> Result<(), JsValue>;

    pub type EncodedVideoChunk;
    #[wasm_bindgen(constructor, catch)]
    pub fn new(init: &Object) -> Result<EncodedVideoChunk, JsValue>;
    #[wasm_bindgen(method, getter, js_name = byteLength)]
    pub fn byte_length(this: &EncodedVideoChunk) -> u32;
    #[wasm_bindgen(method, getter)]
    pub fn timestamp(this: &EncodedVideoChunk) -> f64;
    #[wasm_bindgen(method, getter, js_name = type)]
    pub fn kind(this: &EncodedVideoChunk) -> String;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    pub fn copy_to(this: &EncodedVideoChunk, destination: &Uint8Array) -> Result<(), JsValue>;

    /// A canvas's 2D context, for the one call web-sys has no overload of:
    /// drawing a decoded `VideoFrame`.
    pub type FrameCanvas;
    #[wasm_bindgen(method, catch, js_name = drawImage)]
    pub fn draw_frame(
        this: &FrameCanvas,
        frame: &VideoFrame,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), JsValue>;

    /// An `AudioBuffer`, for its samples as the browser's own array —
    /// web-sys's `get_channel_data` copies them into this module's memory,
    /// which for an hour of music is more than it should ever hold.
    pub type Samples;
    #[wasm_bindgen(method, catch, js_name = getChannelData)]
    pub fn channel(this: &Samples, index: u32) -> Result<js_sys::Float32Array, JsValue>;
}

/// A plain object of `entries` — a WebCodecs configuration or init.
pub fn object(entries: &[(&str, JsValue)]) -> Object {
    let object = Object::new();
    for (key, value) in entries {
        let _ = Reflect::set(&object, &JsValue::from_str(key), value);
    }
    object
}

/// Whether this browser has the class `name` at all.
#[cfg(test)]
pub fn has(name: &str) -> bool {
    Reflect::get(&js_sys::global(), &JsValue::from_str(name))
        .map(|class| class.is_function())
        .unwrap_or(false)
}

/// `name.isConfigSupported(config)`, as a plain yes or no. The class being
/// absent, the call throwing, the promise rejecting, or a result that does
/// not say `supported: true` are all "no".
pub async fn supported(name: &str, config: &Object) -> bool {
    let Ok(class) = Reflect::get(&js_sys::global(), &JsValue::from_str(name)) else {
        return false;
    };
    if !class.is_function() {
        return false;
    }
    let Some(check) = Reflect::get(&class, &JsValue::from_str("isConfigSupported"))
        .ok()
        .and_then(|check| check.dyn_into::<Function>().ok())
    else {
        return false;
    };
    let Ok(promise) = check.call1(&class, config) else {
        return false;
    };
    let Ok(promise) = promise.dyn_into::<Promise>() else {
        return false;
    };
    match JsFuture::from(promise).await {
        Ok(answer) => Reflect::get(&answer, &JsValue::from_str("supported"))
            .ok()
            .and_then(|supported| supported.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// AAC-LC — `mp4a.40.2` — at `sample_rate` with `channels`, at `bitrate`.
pub fn aac_config(sample_rate: u32, channels: u32, bitrate: u64) -> Object {
    object(&[
        ("codec", JsValue::from_str("mp4a.40.2")),
        ("sampleRate", JsValue::from(sample_rate)),
        ("numberOfChannels", JsValue::from(channels)),
        ("bitrate", JsValue::from(bitrate as f64)),
    ])
}

/// Whether this browser can encode AAC-LC as asked.
pub async fn aac_supported(sample_rate: u32, channels: u32, bitrate: u64) -> bool {
    supported("AudioEncoder", &aac_config(sample_rate, channels, bitrate)).await
}

/// The H.264 encoder configuration for a `width × height` output at
/// `frame_rate` and `bitrate`: High profile where this browser encodes it,
/// Main where it offers nothing else — the protocol's order — and None when
/// it encodes neither. (Baseline is not asked for: the profile names High
/// and Main, and an encoder that offers only Baseline — Firefox's — leaves
/// the file to rule C.)
pub async fn h264_config(width: u32, height: u32, frame_rate: f64, bitrate: u64) -> Option<Object> {
    for profile in [H264Profile::High, H264Profile::Main] {
        let config = object(&[
            (
                "codec",
                JsValue::from_str(&h264_codec(profile, width, height, frame_rate)),
            ),
            ("width", JsValue::from(width)),
            ("height", JsValue::from(height)),
            ("bitrate", JsValue::from(bitrate as f64)),
            ("framerate", JsValue::from(frame_rate)),
            // Quality over speed: the send waits for this, not a live call.
            ("latencyMode", JsValue::from_str("quality")),
            // Length-prefixed access units and an avcC record — what an
            // MP4 carries — not Annex B start codes.
            (
                "avc",
                object(&[("format", JsValue::from_str("avc"))]).into(),
            ),
        ]);
        if supported("VideoEncoder", &config).await {
            return Some(config);
        }
    }
    None
}

/// The bytes of a `BufferSource` — an `ArrayBuffer` or any view on one, as
/// an encoder hands back its `decoderConfig.description`.
pub fn bytes_of(value: &JsValue) -> Option<Vec<u8>> {
    if let Some(buffer) = value.dyn_ref::<js_sys::ArrayBuffer>() {
        return Some(Uint8Array::new(buffer).to_vec());
    }
    if js_sys::ArrayBuffer::is_view(value) {
        let buffer = Reflect::get(value, &JsValue::from_str("buffer")).ok()?;
        let offset = Reflect::get(value, &JsValue::from_str("byteOffset"))
            .ok()?
            .as_f64()?;
        let length = Reflect::get(value, &JsValue::from_str("byteLength"))
            .ok()?
            .as_f64()?;
        let view =
            Uint8Array::new_with_byte_offset_and_length(&buffer, offset as u32, length as u32);
        return Some(view.to_vec());
    }
    None
}

/// `metadata.decoderConfig[key]`, from an encoder's output callback.
pub fn decoder_config_field(metadata: &JsValue, key: &str) -> Option<JsValue> {
    if metadata.is_undefined() || metadata.is_null() {
        return None;
    }
    let config = Reflect::get(metadata, &JsValue::from_str("decoderConfig")).ok()?;
    if config.is_undefined() || config.is_null() {
        return None;
    }
    let value = Reflect::get(&config, &JsValue::from_str(key)).ok()?;
    (!value.is_undefined() && !value.is_null()).then_some(value)
}

/// Standing in for a browser that lacks something, in the tests of every
/// module that asks one what it can do.
#[cfg(test)]
pub mod testing {
    use super::*;

    /// Put `value` at `window[name]` for the length of a test, and the old
    /// one back after — the way a test stands in for a browser without it.
    pub struct Stand {
        name: &'static str,
        was: JsValue,
    }

    impl Stand {
        pub fn in_for(name: &'static str, value: &JsValue) -> Stand {
            let global = js_sys::global();
            let was = Reflect::get(&global, &JsValue::from_str(name)).unwrap();
            Reflect::set(&global, &JsValue::from_str(name), value).unwrap();
            Stand { name, was }
        }
    }

    impl Drop for Stand {
        fn drop(&mut self) {
            let _ = Reflect::set(&js_sys::global(), &JsValue::from_str(self.name), &self.was);
        }
    }

    /// A class whose `isConfigSupported` answers `answer` (or throws).
    pub fn fake_class(body: &str) -> JsValue {
        let make = Function::new_no_args(&format!(
            "const C = function() {{}}; C.isConfigSupported = {body}; return C;"
        ));
        make.call0(&JsValue::NULL).unwrap()
    }

    /// One way for a browser to lack a class — [`without`] it, or
    /// [`refusing`] everything asked of it. A stand-in is made where it is
    /// used, one at a time: two alive at once for the same class put each
    /// other back in the wrong order.
    pub type Lack = fn(&'static str) -> Stand;

    /// A browser with no such class at all.
    pub fn without(name: &'static str) -> Stand {
        Stand::in_for(name, &JsValue::UNDEFINED)
    }

    /// A browser whose `name` answers every configuration "not supported".
    pub fn refusing(name: &'static str) -> Stand {
        Stand::in_for(name, &fake_class("async () => ({supported: false})"))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use wasm_bindgen_test::*;

    /// The branch this browser takes is whatever it says, not what its name
    /// suggests — and a browser that says "no" in any of the ways a browser
    /// can is a "no", never an exception.
    #[wasm_bindgen_test]
    async fn support_is_what_the_browser_answers_and_every_failure_is_no() {
        {
            let _gone = Stand::in_for("AudioEncoder", &JsValue::UNDEFINED);
            assert!(!has("AudioEncoder"));
            assert!(!aac_supported(48_000, 1, 64_000).await, "no class");
        }
        {
            let _no = Stand::in_for(
                "AudioEncoder",
                &fake_class("async () => ({supported: false})"),
            );
            assert!(!aac_supported(48_000, 1, 64_000).await);
        }
        {
            let _yes = Stand::in_for(
                "AudioEncoder",
                &fake_class("async () => ({supported: true})"),
            );
            assert!(aac_supported(48_000, 1, 64_000).await);
        }
        {
            let _throws = Stand::in_for(
                "AudioEncoder",
                &fake_class("() => { throw new Error('no'); }"),
            );
            assert!(!aac_supported(48_000, 1, 64_000).await);
        }
        {
            let _rejects = Stand::in_for(
                "AudioEncoder",
                &fake_class("() => Promise.reject(new Error('no'))"),
            );
            assert!(!aac_supported(48_000, 1, 64_000).await);
        }
        {
            let _odd = Stand::in_for(
                "AudioEncoder",
                &fake_class("async () => ({supported: 'yes'})"),
            );
            assert!(!aac_supported(48_000, 1, 64_000).await, "only a real true");
        }
        {
            let _none = Stand::in_for(
                "VideoEncoder",
                &fake_class("async () => ({supported: false})"),
            );
            assert!(h264_config(1280, 720, 30.0, 2_000_000).await.is_none());
        }
        {
            // Main only: High is refused, Main is taken.
            let _main = Stand::in_for(
                "VideoEncoder",
                &fake_class("async (c) => ({supported: c.codec.startsWith('avc1.4d')})"),
            );
            let config = h264_config(1280, 720, 30.0, 2_000_000).await.unwrap();
            let codec = Reflect::get(&config, &JsValue::from_str("codec")).unwrap();
            assert_eq!(codec.as_string().as_deref(), Some("avc1.4d401f"));
        }
    }

    /// And in THIS browser, the real answers the tests below rely on — so a
    /// test runner without them fails here, by name, instead of passing a
    /// transcode test that never transcoded.
    #[wasm_bindgen_test]
    async fn the_test_browser_encodes_what_the_profile_asks_for() {
        assert!(has("AudioEncoder") && has("VideoEncoder") && has("VideoDecoder"));
        assert!(aac_supported(48_000, 1, 64_000).await, "voice notes");
        assert!(aac_supported(44_100, 2, 128_000).await);
        let config = h264_config(1280, 720, 30.0, 2_000_000).await.unwrap();
        let codec = Reflect::get(&config, &JsValue::from_str("codec")).unwrap();
        assert_eq!(codec.as_string().as_deref(), Some("avc1.64001f"), "High");
    }

    #[wasm_bindgen_test]
    fn a_buffer_source_is_read_whatever_its_shape() {
        let bytes = Uint8Array::from(&[1u8, 2, 3, 4, 5][..]);
        assert_eq!(bytes_of(&bytes.buffer().into()), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(bytes_of(&bytes.subarray(1, 4).into()), Some(vec![2, 3, 4]));
        let view = js_sys::DataView::new(&bytes.buffer(), 2, 2);
        assert_eq!(bytes_of(&view.into()), Some(vec![3, 4]));
        assert_eq!(bytes_of(&JsValue::from_str("no")), None);
    }
}
