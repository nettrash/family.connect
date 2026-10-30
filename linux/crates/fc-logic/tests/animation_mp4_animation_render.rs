//! `AnimationRenderTests`, the part about the MP4's sample table made exact afterwards (issue #96;
//! docs/protocol.md, "Animations", "The file") — `AnimationMp4` and the `Faststart` that follows
//! it. The rest of that C# file (drawing on white, area averaging, BT.709 NV12, the frames in the
//! plan's order) is `AnimationRender`'s and `AnimationFrames`'s, in `animation_frames_animation_render.rs`.

use fc_logic::{animation_mp4, faststart};

fn bx(kind: &str, children: &[&[u8]]) -> Vec<u8> {
    let body: Vec<u8> = children.concat();
    let mut b = Vec::with_capacity(8 + body.len());
    b.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    b.extend_from_slice(kind.as_bytes());
    b.extend_from_slice(&body);
    b
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

/// What an encoder might write: ftyp, the media data, then the index — a 1000-a-second movie, a
/// 10 000 000-a-second track (hundreds of nanoseconds, not a multiple of 30), one sample per
/// entry, an edit list, and a ctts.
fn encoded(
    track_scale: u32,
    durations: &[u32],
    ctts: bool,
    handler: &str,
    tracks: usize,
) -> Vec<u8> {
    let ftyp = bx("ftyp", &[b"mp42\0\0\0\0mp42isom"]);
    let mdat = bx("mdat", &[&(0..64).map(|at| at as u8).collect::<Vec<u8>>()]);
    let total = durations
        .iter()
        .fold(0u32, |sum, value| sum.wrapping_add(*value));
    let mvhd = bx("mvhd", &[&u32s(&[0, 0, 0, 1000, 999]), &[0; 80]]);
    let count = durations.len() as u32;
    let stts: Vec<u32> = [0, count]
        .into_iter()
        .chain(durations.iter().flat_map(|value| [1, *value]))
        .collect();
    let ctts_box = if ctts {
        let table: Vec<u32> = [0, count]
            .into_iter()
            .chain(durations.iter().flat_map(|value| [1, value * 2]))
            .collect();
        bx("ctts", &[&u32s(&table)])
    } else {
        Vec::new()
    };
    let trak = bx(
        "trak",
        &[
            &bx("tkhd", &[&u32s(&[0, 0, 0, 1, 0, 999]), &[0; 60]]),
            &bx(
                "edts",
                &[&bx("elst", &[&u32s(&[0, 1, 999, 0, 0x0001_0000])])],
            ),
            &bx(
                "mdia",
                &[
                    &bx("mdhd", &[&u32s(&[0, 0, 0, track_scale, total]), &[0; 4]]),
                    &bx("hdlr", &[&u32s(&[0, 0]), handler.as_bytes(), &[0; 13]]),
                    &bx(
                        "minf",
                        &[&bx(
                            "stbl",
                            &[
                                &bx("stsd", &[&u32s(&[0, 0])]),
                                &bx("stts", &[&u32s(&stts)]),
                                &ctts_box,
                                &bx("stsz", &[&u32s(&[0, 0, count])]),
                                &bx("stco", &[&u32s(&[0, 1, (ftyp.len() + 8) as u32])]),
                            ],
                        )],
                    ),
                ],
            ),
        ],
    );
    let mut children = vec![mvhd];
    children.extend((0..tracks).map(|_| trak.clone()));
    let refs: Vec<&[u8]> = children.iter().map(Vec::as_slice).collect();
    let moov = bx("moov", &refs);
    [ftyp, mdat, moov].concat()
}

fn video(track_scale: u32, durations: &[u32]) -> Vec<u8> {
    encoded(track_scale, durations, true, "vide", 1)
}

fn read_u32(file: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(file[at..at + 4].try_into().unwrap())
}

fn read(file: &[u8], path: &str, offset: usize) -> u32 {
    let mut at = 0;
    let mut end = file.len();
    for kind in path.split('/') {
        while at < end && &file[at + 4..at + 8] != kind.as_bytes() {
            at += read_u32(file, at) as usize;
        }
        assert!(at < end, "no {kind} in {path}");
        end = at + read_u32(file, at) as usize;
        at += 8;
    }
    read_u32(file, at + offset)
}

#[test]
fn a_timescale_that_is_not_a_multiple_of_thirty_becomes_thirty_thousand() {
    let file = video(10_000_000, &[333_333, 1_000_000, 333_334]);
    let exact = animation_mp4::exact_ticks(&file, &[3, 30, 1]).expect("made exact");
    let (scale, durations) = animation_mp4::timing(&exact).expect("read back");
    assert_eq!(scale, 30_000);
    assert_eq!(durations, [3_000, 30_000, 1_000]);
    assert_eq!(read(&exact, "moov/trak/mdia/mdhd", 16), 34_000);
    // The movie's 1000-a-second durations: 34 ticks is 1133⅓ ms.
    assert_eq!(read(&exact, "moov/mvhd", 16), 1133);
    assert_eq!(read(&exact, "moov/trak/tkhd", 20), 1133);
    assert_eq!(read(&exact, "moov/trak/edts/elst", 8), 1133);
    // Composition offsets carried into the new timescale: 666 666 hundreds of ns is 2 000
    // thirty-thousandths.
    assert_eq!(read(&exact, "moov/trak/mdia/minf/stbl/ctts", 12), 2_000);
    // The media data did not move, and the faststart that follows re-points it.
    assert_eq!(file[..8 + 16 + 72], exact[..8 + 16 + 72]);
    let moved =
        animation_mp4::timing(&faststart::moov_first(&exact).expect("moved")).expect("read back");
    assert_eq!(moved.0, 30_000);
    assert_eq!(moved.1, durations);
}

/// A timescale that is already a multiple of 30 is kept; runs of equal samples are one stts entry.
#[test]
fn a_multiple_of_thirty_is_kept_and_runs_are_one_entry() {
    let exact = animation_mp4::exact_ticks(
        &encoded(90_000, &[9_000, 9_000, 9_000, 90_000], false, "vide", 1),
        &[3, 3, 3, 30],
    )
    .expect("made exact");
    let (scale, durations) = animation_mp4::timing(&exact).expect("read back");
    assert_eq!(scale, 90_000);
    assert_eq!(durations, [9_000, 9_000, 9_000, 90_000]);
    assert_eq!(read(&exact, "moov/trak/mdia/minf/stbl/stts", 4), 2);
    assert_eq!(read(&exact, "moov/trak/mdia/minf/stbl/stts", 8), 3);
}

/// What it cannot vouch for, it leaves alone: the encoder's own timing goes, which plays the same.
#[test]
fn what_it_cannot_vouch_for_it_leaves_alone() {
    // A sample count that is not the plan's: a frame dropped or doubled.
    assert!(
        animation_mp4::exact_ticks(&video(10_000_000, &[333_333, 333_333]), &[1, 1, 1]).is_none()
    );
    // An index already in front of the media data, whose offsets a resize would break.
    let in_front = faststart::moov_first(&video(10_000_000, &[333_333]))
        .expect("moved")
        .into_owned();
    assert!(animation_mp4::exact_ticks(&in_front, &[1]).is_none());
    // Not a video track, or more than one track.
    assert!(
        animation_mp4::exact_ticks(&encoded(10_000_000, &[333_333], true, "soun", 1), &[1])
            .is_none()
    );
    assert!(
        animation_mp4::exact_ticks(&encoded(10_000_000, &[333_333], true, "vide", 2), &[1])
            .is_none()
    );
    // No ticks, or bytes that are no MP4.
    assert!(animation_mp4::exact_ticks(&video(10_000_000, &[333_333]), &[]).is_none());
    assert!(animation_mp4::exact_ticks(&[1, 2, 3], &[1]).is_none());
}

/// Not in the C#: a `tkhd` with no room for its duration — which threw `IndexOutOfRangeException`
/// out of `ExactTicks` when it was empty (measured on .NET 10) — is left alone like any other box
/// it cannot vouch for.
#[test]
fn a_track_header_with_no_room_for_its_duration_is_left_alone() {
    let file = video(10_000_000, &[333_333]);
    let tkhd = file
        .windows(4)
        .position(|window| window == b"tkhd")
        .expect("a tkhd")
        - 4;
    let size = read_u32(&file, tkhd) as usize;
    for keep in [0, 1, 20, 24] {
        // The same file with the tkhd's payload cut to `keep` bytes, every enclosing size adjusted.
        let cut = size - 8 - keep;
        let mut shorter = [&file[..tkhd + 8 + keep], &file[tkhd + size..]].concat();
        shorter[tkhd..tkhd + 4].copy_from_slice(&((8 + keep) as u32).to_be_bytes());
        for parent in ["moov", "trak"] {
            let at = shorter
                .windows(4)
                .position(|window| window == parent.as_bytes())
                .expect("a parent")
                - 4;
            let grown = read_u32(&shorter, at) - cut as u32;
            shorter[at..at + 4].copy_from_slice(&grown.to_be_bytes());
        }
        // 24 bytes hold the duration (version 0): the cut file is otherwise sound.
        assert_eq!(
            animation_mp4::exact_ticks(&shorter, &[1]).is_some(),
            keep == 24,
            "{keep}"
        );
    }
}
