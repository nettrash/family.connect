//! `TranscriptSoundTests`: the sound a device sends with a transcript request when the server
//! cannot send its own copy (docs/protocol.md, "Transcripts on request", the multipart form) —
//! which way it is made (the AAC track passed through, or encoded at 64 kbit/s mono), what bounds
//! it, and what is offered at all.
//!
//! ADAPTED, ON PURPOSE: Windows' attempts end with a second encode at Media Foundation's nearest
//! documented rate (96 000), because that encoder refuses 64 000 for mono. GStreamer's `avenc_aac`
//! takes 64 000 as asked (linux/docs/carry-forward.md: "AAC 64 000 accepted as asked, no rate
//! ladder"), so on Linux there is ONE encode: `anything_but_aac_is_reencoded` expects it alone, and
//! `unknown_numbers_are_tried_and_measured` counts two attempts where the C# counts three.

mod common;

use fc_core::protocol::{ApiError, error_codes};
use fc_logic::media_encoding::AacEncoding;
use fc_logic::transcode_attempts::AttemptEnd;
use fc_logic::transcript_sound::{self, SoundAttempt, SoundWay};
use fc_logic::transcripts::transcript_rules;

const MIB25: i64 = 26_214_400;

fn aac(sample_rate: u32, channels: u32, bitrate: u32) -> AacEncoding {
    AacEncoding {
        sample_rate,
        channels,
        bitrate,
    }
}

/// PASS THROUGH WHAT IS ALREADY AAC: a phone's video carries AAC, and the first way of asking is
/// to copy that track untouched — then the voice-note encode, should the copy be refused or come
/// out wrong.
#[test]
fn an_aac_track_is_passed_through_first() {
    let attempts = transcript_sound::attempts(
        Some("aac"),
        Some(128_000),
        Some(48_000),
        Some(60_000),
        MIB25,
    );
    assert_eq!(SoundAttempt::PASSTHROUGH, attempts[0]);
    assert_eq!(None, attempts[0].aac);
    assert!(
        attempts[1..]
            .iter()
            .all(|attempt| attempt.way == SoundWay::Reencode)
    );
    // The encode keeps 48 kHz as 48, and is MONO at 64 kbit/s first.
    assert_eq!(Some(aac(48_000, 1, 64_000)), attempts[1].aac);
}

/// RE-ENCODE EVERYTHING ELSE — Opus or Vorbis in an Ogg, MP3, PCM, and a codec nobody can name — at
/// a voice note's numbers: AAC-LC, mono, 64 kbit/s. Never a passthrough. (The C#'s theory, one row
/// per codec; its second encode at 96 000 is Media Foundation's and not Linux's.)
#[test]
fn anything_but_aac_is_reencoded() {
    for codec in ["unknown", "mp3", "pcm", "flac", "alac"] {
        let attempts = transcript_sound::attempts(
            Some(codec),
            Some(320_000),
            Some(44_100),
            Some(60_000),
            MIB25,
        );
        assert_eq!(
            vec![SoundAttempt {
                way: SoundWay::Reencode,
                aac: Some(aac(44_100, 1, 64_000)),
            }],
            attempts,
            "{codec}"
        );
    }
}

/// NO SOUND TRACK, NOTHING TO SEND: no attempt at all, which the device says as "not available".
#[test]
fn no_track_is_no_attempt() {
    assert!(transcript_sound::attempts(None, None, None, Some(60_000), MIB25).is_empty());
}

/// THE SIZE BOUND decides what is tried: a copy whose own rate would be over the ceiling for this
/// length is skipped for the encode; an encode that would be over too is skipped; and a length
/// nothing can fit tries nothing.
#[test]
fn what_cannot_fit_is_not_tried() {
    // 20 minutes of 256 kbit/s AAC is ~38 MB: too big to copy, and 64 kbit/s (~9.8 MB) fits.
    let twenty = 20 * 60_000;
    let attempts = transcript_sound::attempts(
        Some("aac"),
        Some(256_000),
        Some(44_100),
        Some(twenty),
        MIB25,
    );
    assert!(!attempts.contains(&SoundAttempt::PASSTHROUGH));
    assert_eq!(64_000, attempts[0].aac.expect("an encode").bitrate);

    // 40 minutes: 64 kbit/s fits (~19.6 MB).
    let forty = 40 * 60_000;
    assert_eq!(
        vec![SoundAttempt {
            way: SoundWay::Reencode,
            aac: Some(aac(44_100, 1, 64_000)),
        }],
        transcript_sound::attempts(Some("mp3"), Some(128_000), Some(44_100), Some(forty), MIB25)
    );

    // An hour does not fit at all, and a lower ceiling the server names is held to as well.
    assert!(
        transcript_sound::attempts(
            Some("aac"),
            Some(64_000),
            Some(44_100),
            Some(60 * 60_000),
            MIB25
        )
        .is_empty()
    );
    assert!(
        transcript_sound::attempts(
            Some("mp3"),
            Some(128_000),
            Some(44_100),
            Some(60_000),
            100_000
        )
        .is_empty()
    );
}

/// A rate or a length nobody states is TRIED, never assumed too big — the result is measured before
/// it goes. (Two attempts on Linux, the passthrough and the one encode; three on Windows.)
#[test]
fn unknown_numbers_are_tried_and_measured() {
    assert_eq!(
        SoundAttempt::PASSTHROUGH,
        transcript_sound::attempts(Some("aac"), None, None, Some(60 * 60_000), MIB25)[0]
    );
    assert_eq!(
        2,
        transcript_sound::attempts(Some("aac"), Some(512_000), None, None, MIB25).len()
    );
}

/// THE ESTIMATE: the sound, a fiftieth for the sample tables and a fixed head — 64 kbit/s is 8 000
/// bytes a second, so fifty minutes is about 24.5 MB and still fits 25 MiB; nonsense in is nothing,
/// not an overflow.
#[test]
fn the_estimate_is_the_sound_and_its_overhead() {
    assert_eq!(
        8_000 + 160 + 16_384,
        transcript_sound::estimated_bytes(64_000, 1_000)
    );
    assert!(transcript_sound::estimated_bytes(64_000, 50 * 60_000) <= MIB25);
    assert!(transcript_sound::estimated_bytes(64_000, 55 * 60_000) > MIB25);
    assert_eq!(0, transcript_sound::estimated_bytes(0, 1_000));
    assert_eq!(0, transcript_sound::estimated_bytes(64_000, -1));
    assert_eq!(
        i64::MAX,
        transcript_sound::estimated_bytes(i64::MAX, i64::MAX)
    );
}

/// HIDDEN ONLY WHEN THE ATTACHMENT ALREADY SAYS NO: a stated length whose 64 kbit/s sound is over
/// the ceiling. A length nobody stated is offered.
#[test]
fn only_a_stated_length_that_cannot_fit_hides_the_action() {
    assert!(transcript_sound::can_fit(Some(50 * 60_000), MIB25));
    assert!(!transcript_sound::can_fit(Some(55 * 60_000), MIB25));
    assert!(!transcript_sound::can_fit(Some(60_000), 100_000));
    assert!(transcript_sound::can_fit(None, MIB25));
    assert!(transcript_sound::can_fit(Some(0), MIB25));
}

/// What may go: something, and never a byte over the ceiling.
#[test]
fn the_made_sound_must_fit_the_ceiling() {
    assert!(transcript_sound::fits(MIB25, MIB25));
    assert!(!transcript_sound::fits(MIB25 + 1, MIB25));
    assert!(!transcript_sound::fits(0, MIB25));
}

/// WHAT CAME OUT, JUDGED: AAC within the ceiling is taken; a copy that came out otherwise, or too
/// big, leaves the encode to try; an encode at the wrong rate leaves the next rate; an encode that
/// is not AAC, or anything carrying a picture, ends the asking — no other way will put that right.
#[test]
fn what_came_out_is_judged() {
    let encode = SoundAttempt {
        way: SoundWay::Reencode,
        aac: Some(aac(44_100, 1, 64_000)),
    };
    let copy = SoundAttempt::PASSTHROUGH;
    let judge = transcript_sound::judge;

    assert_eq!(
        AttemptEnd::Taken,
        judge(&copy, Some("aac"), Some(256_000), 1_000_000, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Taken,
        judge(&encode, Some("aac"), Some(64_000), 1_000_000, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Taken,
        judge(&encode, Some("aac"), None, 1_000_000, false, MIB25)
    );

    assert_eq!(
        AttemptEnd::Refused,
        judge(&copy, Some("mp3"), None, 1_000_000, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Refused,
        judge(&copy, None, None, 1_000_000, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Refused,
        judge(&copy, Some("aac"), Some(256_000), MIB25 + 1, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Refused,
        judge(&encode, Some("aac"), Some(96_000), 1_000_000, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Refused,
        judge(&encode, Some("aac"), Some(64_000), MIB25 + 1, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Refused,
        judge(&encode, Some("aac"), Some(64_000), 0, false, MIB25)
    );

    assert_eq!(
        AttemptEnd::Wrong,
        judge(&encode, Some("pcm"), Some(64_000), 1_000_000, false, MIB25)
    );
    assert_eq!(
        AttemptEnd::Wrong,
        judge(&copy, Some("aac"), Some(64_000), 1_000_000, true, MIB25)
    );
    assert_eq!(
        AttemptEnd::Wrong,
        judge(&encode, Some("aac"), Some(64_000), 1_000_000, true, MIB25)
    );
}

/// The held file's extension for the reader: by the stored type, by the name's own, and `bin`
/// otherwise — never anything from a name but letters and digits, since it becomes part of a path.
/// (The C#'s theory `TheHeldFileIsNamedForMediaFoundation`, one row per case.)
#[test]
fn the_held_file_is_named_for_media_foundation() {
    for (mime, name, extension) in [
        (Some("video/mp4"), None, "mp4"),
        (Some("video/quicktime"), Some("clip.mp4"), "mov"),
        (Some("audio/ogg"), None, "ogg"),
        (Some("audio/ogg; codecs=opus"), None, "ogg"),
        (Some("AUDIO/MPEG"), None, "mp3"),
        (Some("audio/mp4"), None, "m4a"),
        (Some("audio/m4a"), None, "m4a"),
        (Some("audio/wav"), None, "wav"),
        (Some("application/octet-stream"), Some("talk.OPUS"), "opus"),
        (None, Some("talk.flac"), "flac"),
        (None, Some("talk.../../x"), "bin"),
        (None, Some("talk.mp4 x"), "bin"),
        (None, Some("talk.verylong"), "bin"),
        (None, None, "bin"),
    ] {
        assert_eq!(
            extension,
            transcript_sound::temp_extension(mime, name),
            "{mime:?} {name:?}"
        );
    }
}

/// The device's own "cannot" is final, and says which it was: too long even re-encoded, or no sound
/// it could read. "Not available for this message." stays for a recording there is nothing to ask
/// with at all.
#[test]
fn the_devices_cannot_is_final() {
    let say = common::english();
    let cases: [(ApiError, &str); 3] = [
        (
            transcript_sound::too_long(),
            "This recording is too long to turn into text.",
        ),
        (
            transcript_sound::unreadable(),
            "Couldn't read the sound in this file.",
        ),
        (
            transcript_sound::unavailable(),
            "Not available for this message.",
        ),
    ];
    for (error, said) in cases {
        assert!(!error.transient());
        assert!(!transcript_rules::may_retry(&error));
        assert_eq!(said, transcript_rules::failure_sentence(&error, &say));
    }
    assert_eq!(
        error_codes::NOT_TRANSCRIBABLE,
        transcript_sound::unavailable().code
    );
}

/// The part, as the server reads it: one part named `audio`, AAC in MPEG-4.
#[test]
fn the_part_is_an_m4a_named_audio() {
    assert_eq!("audio", transcript_sound::PART_NAME);
    assert_eq!("audio/mp4", transcript_sound::MIME);
    assert_eq!("audio.m4a", transcript_sound::FILE_NAME);
}
