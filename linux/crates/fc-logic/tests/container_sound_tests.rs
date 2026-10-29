//! `ContainerSoundTests`: Rule E's count — the sound tracks that PLAY, read from the container's own
//! track list — and what is decided from it (docs/protocol.md, "Preparing media before upload",
//! **Rule E — a transcode keeps the sound**; issue #98).
//!
//! The files here are built box by box by a writer of this test's own, as `faststart_tests.rs`'
//! are, so the production walker is never the thing that says it walked correctly. Every `tkhd`
//! field is written at the offset ISO/IEC 14496-12 gives it, and the ones before `alternate_group`
//! are filled with non-zero bytes, so a walker reading the group from the wrong version's offset
//! reads a different number. Media Foundation's embedded tracks are, on Linux, the discoverer's
//! audio streams (`DecodedTrack`).

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use fc_logic::container_sound::{
    self, DecodedTrack, MixSound, SoundBefore, SoundTrack, before_transcode, lost_sound,
    mix_came_out, mix_format, mix_map, playing, playing_from, playing_indexes, read,
};

fn boxed(kind: &str, children: &[&[u8]]) -> Vec<u8> {
    let body: Vec<u8> = children.concat();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind.as_bytes());
    out.extend_from_slice(&body);
    out
}

/// A box with a 64-bit size: size field 1, then the real size after the type.
fn large_box(kind: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(kind.as_bytes());
    out.extend_from_slice(&((16 + body.len()) as u64).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// A box whose size field is 0: it runs to the end of the file.
fn to_end_box(kind: &str, body: &[u8]) -> Vec<u8> {
    let mut out = boxed(kind, &[body]);
    out[0..4].copy_from_slice(&0u32.to_be_bytes());
    out
}

/// A track header: version 0 (32-bit times, 84-byte body) or 1 (64-bit times, 96-byte body), with
/// `enabled` as flag 0x1 and `group` as `alternate_group`. Everything before the group is non-zero
/// filler, distinct per `seed`.
fn tkhd(enabled: bool, group: u16, version: u8, seed: u8, id: Option<u32>) -> Vec<u8> {
    let mut body = vec![0u8; if version == 1 { 96 } else { 84 }];
    let group_at = if version == 1 { 46 } else { 34 };
    for (at, byte) in body.iter_mut().enumerate().take(group_at).skip(4) {
        *byte = seed.wrapping_add(at as u8);
    }
    body[0] = version;
    body[3] = if enabled { 0x7 } else { 0x6 }; // in-movie and in-preview set either way: only 0x1 is "enabled"
    body[group_at..group_at + 2].copy_from_slice(&group.to_be_bytes());
    if let Some(track_id) = id {
        let id_at = if version == 1 { 20 } else { 12 };
        body[id_at..id_at + 4].copy_from_slice(&track_id.to_be_bytes());
    }
    boxed("tkhd", &[&body])
}

/// A handler reference: version and flags, pre_defined ("mhlr", as QuickTime writes it), the
/// handler, reserved, a name.
fn hdlr(handler: &str) -> Vec<u8> {
    boxed(
        "hdlr",
        &[
            &[0u8; 4],
            b"mhlr",
            handler.as_bytes(),
            &[0u8; 12],
            b"Core Media\0",
        ],
    )
}

fn trak(handler: &str, enabled: bool, group: u16, version: u8, seed: u8) -> Vec<u8> {
    let stsd = boxed("stsd", &[&[0u8; 8]]);
    let stbl = boxed("stbl", &[&stsd]);
    let minf = boxed("minf", &[&stbl]);
    let mdhd = boxed("mdhd", &[&[0u8; 24]]);
    let mdia = boxed("mdia", &[&mdhd, &hdlr(handler), &minf]);
    boxed("trak", &[&tkhd(enabled, group, version, seed, None), &mdia])
}

/// An ISO `AudioSampleEntry` (QuickTime's sound description version 0): reserved, data reference
/// index, version, revision, vendor, then the channel count at 16 and the rate as 16.16 at 24 — and
/// an `esds` after it.
fn mp4a(channels: u16, rate: u32) -> Vec<u8> {
    let mut body = vec![0u8; 28];
    body[6..8].copy_from_slice(&1u16.to_be_bytes());
    body[16..18].copy_from_slice(&channels.to_be_bytes());
    body[18..20].copy_from_slice(&16u16.to_be_bytes());
    body[24..28].copy_from_slice(&(rate << 16).to_be_bytes());
    boxed("mp4a", &[&body, &boxed("esds", &[&[0u8; 27]])])
}

/// QuickTime's sound description version 2: the rate as a 64-bit float at 32, the channel count as
/// 32 bits at 40.
fn lpcm(channels: u32, rate: f64) -> Vec<u8> {
    let mut body = vec![0u8; 64];
    body[6..8].copy_from_slice(&1u16.to_be_bytes());
    body[8..10].copy_from_slice(&2u16.to_be_bytes());
    body[16..18].copy_from_slice(&3u16.to_be_bytes()); // always 3 in version 2
    body[18..20].copy_from_slice(&16u16.to_be_bytes());
    body[20..22].copy_from_slice(&0xfffeu16.to_be_bytes());
    body[24..28].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // always 1.0 in version 2
    body[28..32].copy_from_slice(&72u32.to_be_bytes());
    body[32..40].copy_from_slice(&rate.to_bits().to_be_bytes());
    body[40..44].copy_from_slice(&channels.to_be_bytes());
    boxed("lpcm", &[&body])
}

fn stsd(entry: &[u8]) -> Vec<u8> {
    boxed("stsd", &[&[0, 0, 0, 0, 0, 0, 0, 1], entry])
}

/// A sound track with its track id and a sample entry that states its channels and rate.
fn voice(id: u32, entry: &[u8], enabled: bool, group: u16) -> Vec<u8> {
    let stbl = boxed("stbl", &[&stsd(entry), &boxed("stts", &[&[0u8; 8]])]);
    let minf = boxed("minf", &[&boxed("smhd", &[&[0u8; 8]]), &stbl]);
    let mdia = boxed(
        "mdia",
        &[&boxed("mdhd", &[&[0u8; 24]]), &hdlr("soun"), &minf],
    );
    boxed("trak", &[&tkhd(enabled, group, 0, 0x11, Some(id)), &mdia])
}

fn voice_on(id: u32, entry: &[u8]) -> Vec<u8> {
    voice(id, entry, true, 0)
}

fn video() -> Vec<u8> {
    trak("vide", true, 0, 0, 0x11)
}

/// `Sound(enabled, group, version, seed)` with the C#'s defaults: enabled, no group, version 0,
/// seed 0x11.
fn sound() -> Vec<u8> {
    trak("soun", true, 0, 0, 0x11)
}

fn sound_with(enabled: bool, group: u16) -> Vec<u8> {
    trak("soun", enabled, group, 0, 0x11)
}

fn sound_version(enabled: bool, group: u16, version: u8, seed: u8) -> Vec<u8> {
    trak("soun", enabled, group, version, seed)
}

fn moov(tracks: &[Vec<u8>]) -> Vec<u8> {
    let mvhd = boxed("mvhd", &[&[0u8; 100]]);
    let mut children: Vec<&[u8]> = vec![&mvhd];
    children.extend(tracks.iter().map(Vec::as_slice));
    boxed("moov", &children)
}

fn ftyp() -> Vec<u8> {
    boxed("ftyp", &[b"isom\0\0\x02\0isomiso2avc1mp41"])
}

fn mdat(length: usize) -> Vec<u8> {
    let body: Vec<u8> = (0..length).map(|at| (at * 7) as u8).collect();
    boxed("mdat", &[&body])
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// A camera's file: the index at the END, after the media data — the common case before faststart.
fn mp4(tracks: &[Vec<u8>]) -> Vec<u8> {
    concat(&[&ftyp(), &mdat(4096), &moov(tracks)])
}

// --- the count ----------------------------------------------------------------------------------

#[test]
fn a_picture_alone_has_no_sound_track() {
    assert_eq!(Some(0), playing(&mp4(&[video()])));
    assert_eq!(Some(0), playing(&mp4(&[])));
}

#[test]
fn one_enabled_sound_track_counts_one() {
    assert_eq!(Some(1), playing(&mp4(&[video(), sound()])));
}

/// A screen recording's app sound and microphone: two voices, which one transcode cannot carry as
/// one track without mixing them.
#[test]
fn two_enabled_sound_tracks_count_two() {
    assert_eq!(Some(2), playing(&mp4(&[video(), sound(), sound()])));
}

/// An iPhone's spatial recording: its stereo AAC and its spatial track, of which a player picks
/// one.
#[test]
fn tracks_in_one_alternate_group_count_once() {
    assert_eq!(
        Some(1),
        playing(&mp4(&[video(), sound_with(true, 1), sound_with(true, 1)]))
    );
    assert_eq!(
        Some(2),
        playing(&mp4(&[
            video(),
            sound_with(true, 1),
            sound_with(true, 1),
            sound_with(true, 2)
        ]))
    );
    assert_eq!(
        Some(2),
        playing(&mp4(&[video(), sound_with(true, 1), sound_with(true, 0)]))
    );
    assert_eq!(
        Some(2),
        playing(&mp4(&[video(), sound_with(true, 0), sound_with(true, 0)]))
    );
}

/// A disabled member of a group is not the group's one: the enabled one is.
#[test]
fn a_disabled_track_is_not_counted() {
    assert_eq!(
        Some(1),
        playing(&mp4(&[video(), sound(), sound_with(false, 0)]))
    );
    assert_eq!(
        Some(1),
        playing(&mp4(&[video(), sound_with(false, 1), sound_with(true, 1)]))
    );
}

/// A player that ignores the flag plays them, so a transcode that drops them has lost something.
#[test]
fn sound_tracks_none_of_which_is_enabled_count_one() {
    assert_eq!(Some(1), playing(&mp4(&[video(), sound_with(false, 0)])));
    assert_eq!(
        Some(1),
        playing(&mp4(&[video(), sound_with(false, 0), sound_with(false, 0)]))
    );
}

/// Version 1's 64-bit times move `alternate_group` from 34 to 46. The filler at 34 differs per
/// track, so reading version 1 at version 0's offset would see two groups — two voices — where
/// there is one.
#[test]
fn a_version_one_track_header_is_read_at_its_own_offsets() {
    assert_eq!(
        Some(1),
        playing(&mp4(&[
            video(),
            sound_version(true, 5, 1, 0x21),
            sound_version(true, 5, 1, 0x42)
        ]))
    );
    assert_eq!(
        Some(2),
        playing(&mp4(&[
            video(),
            sound_version(true, 5, 1, 0x21),
            sound_version(true, 6, 1, 0x21)
        ]))
    );
    assert_eq!(
        Some(1),
        playing(&mp4(&[
            video(),
            sound_version(true, 0, 1, 0x11),
            sound_version(false, 0, 1, 0x11)
        ]))
    );
    // And the two versions side by side, as an editor's re-mux can leave them.
    assert_eq!(
        Some(1),
        playing(&mp4(&[
            video(),
            sound_version(true, 3, 0, 0x11),
            sound_version(true, 3, 1, 0x11)
        ]))
    );
}

#[test]
fn a_track_header_of_an_unknown_version_cannot_be_read() {
    assert_eq!(
        None,
        playing(&mp4(&[video(), sound_version(true, 0, 2, 0x11)]))
    );
}

#[test]
fn sixty_four_bit_box_sizes_are_honoured() {
    assert_eq!(
        Some(2),
        playing(&concat(&[
            &ftyp(),
            &large_box("mdat", &[0u8; 512]),
            &moov(&[video(), sound(), sound()])
        ]))
    );
    assert_eq!(
        Some(1),
        playing(&concat(&[
            &ftyp(),
            &large_box("mdat", &[0u8; 512]),
            &large_box("moov", &moov(&[video(), sound()])[8..])
        ]))
    );
}

#[test]
fn a_moov_that_runs_to_the_end_is_read() {
    assert_eq!(
        Some(1),
        playing(&concat(&[
            &ftyp(),
            &mdat(4096),
            &to_end_box("moov", &moov(&[video(), sound()])[8..])
        ]))
    );
}

#[test]
fn the_moov_is_found_wherever_it_is() {
    assert_eq!(
        Some(2),
        playing(&concat(&[
            &ftyp(),
            &moov(&[video(), sound(), sound()]),
            &mdat(4096)
        ]))
    );
    assert_eq!(
        Some(2),
        playing(&concat(&[
            &ftyp(),
            &boxed("free", &[&[0u8; 16]]),
            &mdat(4096),
            &boxed("wide", &[]),
            &moov(&[video(), sound(), sound()])
        ]))
    );
}

/// QuickTime lets an atom list end in a 32-bit zero; that is not a malformed box.
#[test]
fn a_quick_time_zero_terminator_is_not_a_box() {
    let mvhd = boxed("mvhd", &[&[0u8; 100]]);
    let moov = boxed("moov", &[&mvhd, &video(), &sound(), &[0u8; 4]]);
    assert_eq!(Some(1), playing(&concat(&[&ftyp(), &mdat(4096), &moov])));
}

#[test]
fn what_cannot_be_walked_is_null() {
    assert_eq!(None, playing(&[]));
    assert_eq!(None, playing(b"RIFF....WEBPVP8 not an mp4 at all"));
    let noise: Vec<u8> = (0..300).map(|at: usize| (at * 31 + 5) as u8).collect();
    assert_eq!(None, playing(&noise));
    assert_eq!(None, playing(&concat(&[&ftyp(), &mdat(4096)]))); // no moov
    let file = mp4(&[video(), sound()]);
    assert_eq!(None, playing(&file[..file.len() - 10])); // a moov cut short
    let mut stub = [0u8; 5];
    stub[4] = 0xff;
    let mvhd = boxed("mvhd", &[&[0u8; 100]]);
    assert_eq!(
        None,
        playing(&concat(&[
            &ftyp(),
            &mdat(4096),
            &boxed("moov", &[&mvhd, &sound(), &stub])
        ]))
    );
    let mut overrun = mdat(4096);
    let claimed = (overrun.len() + 1000) as u32;
    overrun[0..4].copy_from_slice(&claimed.to_be_bytes()); // mdat claims past the end
    assert_eq!(
        None,
        playing(&concat(&[&ftyp(), &overrun, &moov(&[sound()])]))
    );
}

/// A track it cannot vouch for makes the whole count unknown, never a guess that might be "none".
#[test]
fn a_track_without_its_header_or_handler_is_null() {
    let mdhd = boxed("mdhd", &[&[0u8; 24]]);
    let no_handler = boxed(
        "trak",
        &[&tkhd(true, 0, 0, 0x11, None), &boxed("mdia", &[&mdhd])],
    );
    let no_header = boxed("trak", &[&boxed("mdia", &[&mdhd, &hdlr("soun")])]);
    let short_header = boxed(
        "trak",
        &[
            &boxed("tkhd", &[&[0u8; 20]]),
            &boxed("mdia", &[&hdlr("soun")]),
        ],
    );
    assert_eq!(None, playing(&mp4(&[video(), no_handler])));
    assert_eq!(None, playing(&mp4(&[video(), no_header])));
    assert_eq!(None, playing(&mp4(&[video(), short_header])));
}

/// A read-only reader over bytes that counts what was read, and can refuse to seek.
struct Counting {
    inner: Cursor<Vec<u8>>,
    seekable: bool,
    bytes_read: u64,
}

impl Counting {
    fn new(bytes: Vec<u8>, seekable: bool) -> Self {
        Counting {
            inner: Cursor::new(bytes),
            seekable,
            bytes_read: 0,
        }
    }
}

impl Read for Counting {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes_read += read as u64;
        Ok(read)
    }
}

impl Seek for Counting {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        if self.seekable {
            self.inner.seek(to)
        } else {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }
}

/// A source can be gigabytes: only the box headers before the index and the index itself are read.
#[test]
fn the_media_data_is_never_read() {
    let mdat = mdat(1_000_000);
    let file = concat(&[&ftyp(), &mdat, &moov(&[video(), sound()])]);
    let length = file.len();
    let mut counted = Counting::new(file, true);
    assert_eq!(Some(1), playing_from(&mut counted).unwrap());
    assert!(
        counted.bytes_read < 4096,
        "read {} bytes of a {length}-byte file",
        counted.bytes_read
    );
}

#[test]
fn a_stream_that_cannot_seek_is_null() {
    let mut forward_only = Counting::new(mp4(&[video(), sound()]), false);
    assert_eq!(None, playing_from(&mut forward_only).unwrap());
}

// --- what Rule E decides ------------------------------------------------------------------------

#[test]
fn one_sound_track_media_foundation_shows_is_transcoded() {
    assert_eq!(SoundBefore::Transcode, before_transcode(Some(1), true));
    assert_eq!(SoundBefore::Transcode, before_transcode(Some(0), false));
    // A container with no sound track and a decoder that shows one: the transcode is checked
    // against the decoder.
    assert_eq!(SoundBefore::Transcode, before_transcode(Some(0), true));
}

/// L1 of issue #98: no audio shown, none asked for, none came out — and the read-back passed it.
#[test]
fn a_sound_track_media_foundation_does_not_show_is_left_alone() {
    assert_eq!(SoundBefore::Unheard, before_transcode(Some(1), false));
}

/// L2 of issue #98, as the owner decided it: more than one voice is MIXED, never thinned to one —
/// and sound the decoder does not show at all is still left alone however many tracks it is on:
/// there is nothing to mix it from.
#[test]
fn more_than_one_sound_track_that_plays_is_mixed() {
    assert_eq!(SoundBefore::Mix, before_transcode(Some(2), true));
    assert_eq!(SoundBefore::Mix, before_transcode(Some(3), true));
    assert_eq!(SoundBefore::Unheard, before_transcode(Some(3), false));
}

/// A count nobody could read changes nothing: the transcode decides as it did before Rule E.
#[test]
fn an_unreadable_count_decides_nothing() {
    assert_eq!(SoundBefore::Transcode, before_transcode(None, true));
    assert_eq!(SoundBefore::Transcode, before_transcode(None, false));
}

#[test]
fn a_result_with_no_sound_from_a_source_with_sound_lost_it() {
    assert!(lost_sound(Some(1), Some(0)));
    assert!(lost_sound(Some(2), Some(0)));
    assert!(!lost_sound(Some(1), Some(1)));
    assert!(
        !lost_sound(Some(0), Some(0)),
        "a silent source makes a silent result"
    );
    assert!(!lost_sound(None, Some(0)), "a source nobody could count");
    assert!(
        !lost_sound(Some(1), None),
        "a result nobody could count: the read-back decides"
    );
}

/// The whole path, end to end on bytes: a source with sound, a result without, counted by the same
/// reader.
#[test]
fn a_silent_transcode_of_a_source_with_sound_is_caught() {
    let source = mp4(&[video(), sound()]);
    let silent = concat(&[&ftyp(), &moov(&[video()]), &mdat(4096)]);
    let kept = concat(&[&ftyp(), &moov(&[video(), sound()]), &mdat(4096)]);
    assert!(lost_sound(playing(&source), playing(&silent)));
    assert!(!lost_sound(playing(&source), playing(&kept)));
}

// --- the track list -----------------------------------------------------------------------------

fn track(
    track_id: u32,
    enabled: bool,
    alternate_group: u16,
    channels: Option<u32>,
    sample_rate: Option<u32>,
) -> SoundTrack {
    SoundTrack {
        track_id,
        enabled,
        alternate_group,
        channels,
        sample_rate,
    }
}

#[test]
fn the_track_list_is_the_sound_tracks_in_moov_order_with_what_their_sample_entries_say() {
    let tracks = read(&mp4(&[
        video(),
        voice_on(2, &mp4a(2, 48_000)),
        voice(7, &mp4a(1, 44_100), false, 3),
    ]));
    assert_eq!(
        Some(vec![
            track(2, true, 0, Some(2), Some(48_000)),
            track(7, false, 3, Some(1), Some(44_100))
        ]),
        tracks
    );
}

#[test]
fn a_quick_time_version_two_sound_description_is_read() {
    assert_eq!(
        Some(vec![track(3, true, 0, Some(6), Some(96_000))]),
        read(&mp4(&[video(), voice_on(3, &lpcm(6, 96_000.0))]))
    );
}

/// A sample entry it cannot read leaves the numbers unknown; the track is still a track.
#[test]
fn an_unreadable_sample_entry_leaves_its_numbers_unknown() {
    assert_eq!(
        Some(vec![track(4, true, 0, None, None)]),
        read(&mp4(&[voice_on(4, &boxed("mp4a", &[&[0u8; 12]]))]))
    );
    assert_eq!(
        vec![None],
        read(&mp4(&[sound()]))
            .unwrap()
            .iter()
            .map(|track| track.channels)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_playing_tracks_are_found_by_position() {
    let t = |enabled: bool, group: u16| track(1, enabled, group, None, None);
    assert_eq!(
        vec![0, 2],
        playing_indexes(&[t(true, 0), t(false, 0), t(true, 0)])
    );
    assert_eq!(
        vec![1, 3],
        playing_indexes(&[t(false, 1), t(true, 1), t(true, 1), t(true, 2)])
    );
    assert_eq!(vec![0], playing_indexes(&[t(false, 0), t(false, 0)]));
    assert!(playing_indexes(&[]).is_empty());
}

// --- the mix ------------------------------------------------------------------------------------

/// `On(channels, rate, enabled, group)` with the C#'s defaults.
fn on(channels: Option<u32>, rate: Option<u32>) -> SoundTrack {
    track(1, true, 0, channels, rate)
}

fn on_with(enabled: bool, group: u16) -> SoundTrack {
    track(1, enabled, group, None, None)
}

fn decoded(channels: Option<u32>, sample_rate: Option<u32>, bitrate: Option<u32>) -> DecodedTrack {
    DecodedTrack {
        channels,
        sample_rate,
        bitrate,
    }
}

fn unstated() -> DecodedTrack {
    DecodedTrack::default()
}

/// A screen recording: the app's sound and the microphone, both read by the decoder, in the same
/// order.
#[test]
fn every_playing_track_is_mixed_when_media_foundation_lists_them_all() {
    assert_eq!(
        Some(vec![0, 1]),
        mix_map(
            &[on(Some(2), Some(48_000)), on(Some(1), Some(48_000))],
            &[
                decoded(Some(2), Some(48_000), Some(128_000)),
                decoded(Some(1), Some(48_000), Some(64_000))
            ]
        )
    );
    // A disabled track is listed by both and mixed by neither.
    assert_eq!(
        Some(vec![0, 2]),
        mix_map(
            &[on(None, None), on_with(false, 0), on(None, None)],
            &[unstated(), unstated(), unstated()]
        )
    );
    // Of an alternate group, its first enabled member only.
    assert_eq!(
        Some(vec![0, 2]),
        mix_map(
            &[on_with(true, 1), on_with(true, 1), on(None, None)],
            &[unstated(), unstated(), unstated()]
        )
    );
}

/// Which one the decoder could not read cannot be told from what it lists, and a mix of the wrong
/// ones keeps fewer voices than the source has: the original goes.
#[test]
fn an_embedded_list_that_is_short_or_long_cannot_be_mapped() {
    let any = on(None, None);
    assert_eq!(None, mix_map(&[any, any, any], &[unstated(), unstated()]));
    assert_eq!(
        None,
        mix_map(&[any, any], &[unstated(), unstated(), unstated()])
    );
    assert_eq!(None, mix_map(&[], &[]));
}

/// Where both sides state a number and it differs, the order is not what it was assumed to be.
#[test]
fn numbers_that_disagree_mean_the_order_is_wrong() {
    assert_eq!(
        None,
        mix_map(
            &[on(Some(2), Some(48_000)), on(Some(1), Some(44_100))],
            &[
                decoded(Some(1), Some(44_100), None),
                decoded(Some(2), Some(48_000), None)
            ]
        )
    );
    assert_eq!(
        None,
        mix_map(
            &[on(Some(2), None), on(Some(2), None)],
            &[decoded(Some(2), None, None), decoded(Some(1), None, None)]
        )
    );
    assert_eq!(
        None,
        mix_map(
            &[on(None, Some(48_000)), on(None, None)],
            &[decoded(None, Some(44_100), None), unstated()]
        )
    );
    // A number only one side states, or one stated as 0, is not a disagreement.
    assert_eq!(
        Some(vec![0, 1]),
        mix_map(
            &[on(Some(2), None), on(None, Some(44_100))],
            &[
                decoded(None, Some(48_000), None),
                decoded(Some(0), Some(0), Some(0))
            ]
        )
    );
}

/// "Stereo when any of them is, at the highest sample rate any of them has" — capped by their rates
/// together.
#[test]
fn a_mix_is_stereo_when_any_track_is_at_the_highest_rate_any() {
    let pair = |decoded: DecodedTrack| (decoded, on(None, None));
    let mix = |channels, sample_rate, bitrate| MixSound {
        channels,
        sample_rate,
        bitrate,
    };
    assert_eq!(
        mix(Some(2), Some(48_000), Some(192_000)),
        mix_format([
            pair(decoded(Some(1), Some(44_100), Some(64_000))),
            pair(decoded(Some(2), Some(48_000), Some(128_000)))
        ])
    );
    assert_eq!(
        mix(Some(1), Some(44_100), Some(128_000)),
        mix_format([
            pair(decoded(Some(1), Some(44_100), Some(64_000))),
            pair(decoded(Some(1), Some(44_100), Some(64_000)))
        ])
    );
    assert_eq!(
        mix(Some(2), None, None),
        mix_format([pair(decoded(Some(6), None, None)), pair(unstated())])
    );
    // Mono beside a count nobody states is not known to be mono.
    assert_eq!(
        mix(None, Some(48_000), None),
        mix_format([
            pair(decoded(Some(1), Some(48_000), Some(64_000))),
            pair(decoded(None, None, None))
        ])
    );
    // The container's numbers where the decoder leaves one out; never its bit rate, which it does
    // not state.
    assert_eq!(
        mix(Some(2), Some(48_000), None),
        mix_format([
            (
                decoded(Some(0), Some(0), Some(0)),
                on(Some(2), Some(48_000))
            ),
            pair(decoded(Some(1), Some(44_100), Some(64_000)))
        ])
    );
}

#[test]
fn a_mix_must_come_out_as_exactly_one_sound_track() {
    assert!(mix_came_out(Some(1)));
    assert!(!mix_came_out(Some(0)), "the mix lost the sound");
    assert!(
        !mix_came_out(Some(2)),
        "the voices came out side by side, not mixed"
    );
    assert!(!mix_came_out(None), "a mix nobody can check is not sent");
}

/// The whole mapping, end to end on bytes: what the container lists, beside what the decoder reads.
#[test]
fn a_screen_recordings_two_voices_are_mapped_from_its_bytes() {
    let tracks = read(&mp4(&[
        video(),
        voice_on(2, &mp4a(2, 48_000)),
        voice_on(3, &mp4a(1, 48_000)),
    ]))
    .unwrap();
    let decoded = [
        decoded(Some(2), Some(48_000), Some(128_000)),
        decoded(Some(1), Some(48_000), Some(64_000)),
    ];
    let mixed = mix_map(&tracks, &decoded);
    assert_eq!(Some(vec![0, 1]), mixed);
    assert_eq!(
        MixSound {
            channels: Some(2),
            sample_rate: Some(48_000),
            bitrate: Some(192_000)
        },
        mix_format(mixed.unwrap().iter().map(|&at| (decoded[at], tracks[at])))
    );
}

/// Not a C# test: the reader over a file is the reader over its bytes, and a read that fails is
/// the caller's error, not a `None` (the C#'s "a read that throws is the caller's to catch").
#[test]
fn a_read_that_fails_is_an_error_not_an_unknown() {
    let file = mp4(&[video(), sound(), sound()]);
    assert_eq!(
        Some(2),
        container_sound::read_from(&mut Cursor::new(file.clone()))
            .unwrap()
            .map(|tracks| tracks.len())
    );
    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }
    impl Seek for Failing {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Ok(4096)
        }
    }
    assert!(container_sound::read_from(&mut Failing).is_err());
}
