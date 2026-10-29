//! `FaststartTests`: the `moov` moved in front of the media data, with every chunk offset that
//! moved re-pointed — checked the only way that matters: every chunk the index names is the same
//! bytes after the move as before it.
//!
//! The files here are built box by box by a writer of this test's own, so the production walker
//! is never the thing that says it walked correctly.

use std::borrow::Cow;

use fc_logic::faststart;

fn bx(kind: &str, children: &[&[u8]]) -> Vec<u8> {
    let body: Vec<u8> = children.concat();
    let mut b = Vec::with_capacity(8 + body.len());
    b.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    b.extend_from_slice(kind.as_bytes());
    b.extend_from_slice(&body);
    b
}

fn full(kind: &str, body: &[u8]) -> Vec<u8> {
    bx(kind, &[&[0; 4], body])
}

fn stco(offsets: &[u32]) -> Vec<u8> {
    let mut body = (offsets.len() as u32).to_be_bytes().to_vec();
    for offset in offsets {
        body.extend_from_slice(&offset.to_be_bytes());
    }
    full("stco", &body)
}

fn co64(offsets: &[u64]) -> Vec<u8> {
    let mut body = (offsets.len() as u32).to_be_bytes().to_vec();
    for offset in offsets {
        body.extend_from_slice(&offset.to_be_bytes());
    }
    full("co64", &body)
}

fn track(table: &[u8]) -> Vec<u8> {
    bx(
        "trak",
        &[
            &bx("tkhd", &[&[0; 84]]),
            &bx(
                "mdia",
                &[
                    &bx("mdhd", &[&[0; 24]]),
                    &bx("minf", &[&bx("stbl", &[&bx("stsd", &[&[0; 8]]), table])]),
                ],
            ),
        ],
    )
}

fn moov_box(tables: &[Vec<u8>]) -> Vec<u8> {
    let mut children = vec![bx("mvhd", &[&[0; 100]])];
    children.extend(tables.iter().map(|table| track(table)));
    let refs: Vec<&[u8]> = children.iter().map(Vec::as_slice).collect();
    bx("moov", &refs)
}

fn ftyp() -> Vec<u8> {
    bx("ftyp", &[b"isom\0\0\x02\0isomiso2avc1mp41"])
}

fn payload(length: usize, seed: usize) -> Vec<u8> {
    (0..length).map(|at| ((at * 7) + seed) as u8).collect()
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn read_u32(file: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(file[at..at + 4].try_into().unwrap())
}

fn read_u64(file: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(file[at..at + 8].try_into().unwrap())
}

/// The top-level box types in order, read by this test's own walker.
fn top_level(file: &[u8]) -> Vec<String> {
    let mut types = Vec::new();
    let mut at = 0;
    while at < file.len() {
        let size = read_u32(file, at) as usize;
        types.push(String::from_utf8(file[at + 4..at + 8].to_vec()).unwrap());
        at += size;
    }
    types
}

fn index_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Every offset in every table named `kind`, found by its four letters.
fn offsets(file: &[u8], kind: &str) -> Vec<u64> {
    let mut offsets = Vec::new();
    let name = kind.as_bytes();
    let mut found = index_of(file, name);
    while let Some(at) = found {
        let count = read_u32(file, at + 8);
        for entry in 0..count as usize {
            offsets.push(if kind == "stco" {
                u64::from(read_u32(file, at + 12 + (4 * entry)))
            } else {
                read_u64(file, at + 12 + (8 * entry))
            });
        }
        found = index_of(&file[at + 4..], name).map(|next| at + 4 + next);
    }
    offsets
}

/// What an offset points at: the sixteen bytes a decoder would read there.
fn at(file: &[u8], offset: u64) -> Vec<u8> {
    file[offset as usize..offset as usize + 16].to_vec()
}

fn arranged(file: &[u8]) -> Vec<u8> {
    faststart::moov_first(file)
        .expect("the index can be moved")
        .into_owned()
}

#[test]
fn the_index_moves_to_the_front_and_every_chunk_is_still_where_it_points() {
    let ftyp = ftyp();
    let payload = payload(4096, 3);
    let mdat_start = ftyp.len() as u32;
    let data = mdat_start + 8;
    // A video track with 32-bit offsets and an audio track with 64-bit ones, both into the one mdat.
    let d = u64::from(data);
    let moov = moov_box(&[
        stco(&[data, data + 100, data + 1000]),
        co64(&[d + 50, d + 3000]),
    ]);
    let file = concat(&[&ftyp, &bx("mdat", &[&payload]), &moov]);
    assert_eq!(faststart::is_moov_first(&file), Some(false));

    let arranged = arranged(&file);

    assert_eq!(top_level(&arranged), ["ftyp", "moov", "mdat"]);
    assert_eq!(arranged.len(), file.len());
    assert_eq!(faststart::is_moov_first(&arranged), Some(true));
    let before: Vec<u64> = offsets(&file, "stco")
        .into_iter()
        .chain(offsets(&file, "co64"))
        .collect();
    let after: Vec<u64> = offsets(&arranged, "stco")
        .into_iter()
        .chain(offsets(&arranged, "co64"))
        .collect();
    assert_eq!(
        before
            .iter()
            .map(|offset| offset + moov.len() as u64)
            .collect::<Vec<_>>(),
        after
    );
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(at(&file, *b), at(&arranged, *a));
    }
    // Nothing else in the index changed: it is byte for byte the index this test would write with
    // the new offsets.
    let by = moov.len() as u32;
    let expected = moov_box(&[
        stco(&[data + by, data + 100 + by, data + 1000 + by]),
        co64(&[d + 50 + u64::from(by), d + 3000 + u64::from(by)]),
    ]);
    assert_eq!(expected, arranged[ftyp.len()..ftyp.len() + moov.len()]);
}

#[test]
fn a_file_already_in_order_is_handed_back_as_it_is() {
    let ftyp = ftyp();
    let moov_length = moov_box(&[stco(&[0])]).len();
    let data = (ftyp.len() + moov_length + 8) as u32;
    let file = concat(&[
        &ftyp,
        &moov_box(&[stco(&[data])]),
        &bx("mdat", &[&payload(256, 1)]),
    ]);
    assert_eq!(faststart::is_moov_first(&file), Some(true));
    // The same bytes, not a copy.
    match faststart::moov_first(&file) {
        Some(Cow::Borrowed(same)) => assert!(std::ptr::eq(same, file.as_slice())),
        other => panic!("not handed back: {:?}", other.map(|_| ())),
    }
}

/// Filler boxes keep their order, and slide like the data does.
#[test]
fn filler_before_the_data_stays_before_it() {
    let ftyp = ftyp();
    let free = bx("free", &[&[0; 32]]);
    let data = (ftyp.len() + free.len() + 8) as u32;
    let file = concat(&[
        &ftyp,
        &free,
        &bx("mdat", &[&payload(512, 9)]),
        &moov_box(&[stco(&[data, data + 256])]),
    ]);
    let arranged = arranged(&file);
    assert_eq!(top_level(&arranged), ["ftyp", "free", "moov", "mdat"]);
    let before = offsets(&file, "stco");
    let after = offsets(&arranged, "stco");
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(at(&file, *b), at(&arranged, *a));
    }
}

/// Only what slid is re-pointed: an index between two data boxes moves in front of the first, the
/// first slides by its size, and the second — before and after, behind both — stays exactly where
/// it was.
#[test]
fn data_after_the_index_is_not_repointed() {
    let ftyp = ftyp();
    let first = bx("mdat", &[&payload(300, 5)]);
    let moov_length = moov_box(&[stco(&[0, 0])]).len();
    let in_first = (ftyp.len() + 8 + 10) as u32;
    let in_second = (ftyp.len() + first.len() + moov_length + 8 + 20) as u32;
    let moov = moov_box(&[stco(&[in_first, in_second])]);
    let file = concat(&[&ftyp, &first, &moov, &bx("mdat", &[&payload(300, 77)])]);
    let arranged = arranged(&file);
    assert_eq!(top_level(&arranged), ["ftyp", "moov", "mdat", "mdat"]);
    assert_eq!(
        offsets(&arranged, "stco"),
        [
            u64::from(in_first) + moov.len() as u64,
            u64::from(in_second)
        ]
    );
    assert_eq!(
        at(&file, u64::from(in_first)),
        at(&arranged, u64::from(in_first) + moov.len() as u64)
    );
    assert_eq!(
        at(&file, u64::from(in_second)),
        at(&arranged, u64::from(in_second))
    );
}

/// A size of 0 means "to the end of the file" — which stops being true once it is not at the end.
#[test]
fn an_index_that_ran_to_the_end_is_given_its_real_size() {
    let ftyp = ftyp();
    let data = (ftyp.len() + 8) as u32;
    let moov = moov_box(&[stco(&[data])]);
    let mut open = moov.clone();
    open[..4].copy_from_slice(&0u32.to_be_bytes());
    let file = concat(&[&ftyp, &bx("mdat", &[&payload(128, 2)]), &open]);
    let arranged = arranged(&file);
    assert_eq!(read_u32(&arranged, ftyp.len()), moov.len() as u32);
    assert_eq!(top_level(&arranged), ["ftyp", "moov", "mdat"]);
    assert_eq!(
        at(&file, u64::from(data)),
        at(&arranged, offsets(&arranged, "stco")[0])
    );
}

/// A 64-bit box size — size 1, the real one after the type — is walked like any other.
#[test]
fn a_large_size_data_box_is_walked() {
    let ftyp = ftyp();
    let payload = payload(200, 4);
    let mut large = vec![0u8; 16 + payload.len()];
    large[..4].copy_from_slice(&1u32.to_be_bytes());
    large[4..8].copy_from_slice(b"mdat");
    let length = large.len() as u64;
    large[8..16].copy_from_slice(&length.to_be_bytes());
    large[16..].copy_from_slice(&payload);
    let data = (ftyp.len() + 16) as u32;
    let file = concat(&[&ftyp, &large, &moov_box(&[stco(&[data, data + 100])])]);
    let arranged = arranged(&file);
    assert_eq!(faststart::is_moov_first(&arranged), Some(true));
    for (b, a) in offsets(&file, "stco")
        .iter()
        .zip(&offsets(&arranged, "stco"))
    {
        assert_eq!(at(&file, *b), at(&arranged, *a));
    }
}

/// What it cannot vouch for, it does not touch.
#[test]
fn anything_it_cannot_vouch_for_is_left_alone() {
    let ftyp = ftyp();
    let data = (ftyp.len() + 8) as u32;
    let mdat = bx("mdat", &[&payload(64, 1)]);
    let moov = moov_box(&[stco(&[data])]);

    // A top-level box that may hold absolute positions of its own, when the index would have to
    // move past it.
    assert!(
        faststart::moov_first(&concat(&[&ftyp, &mdat, &bx("meta", &[&[0; 12]]), &moov])).is_none()
    );
    assert!(
        faststart::moov_first(&concat(&[&ftyp, &mdat, &moov, &bx("moof", &[&[0; 8]])])).is_none()
    );
    // …though a file already in order is fine whatever else it holds.
    let in_order = concat(&[
        &ftyp,
        &moov_box(&[stco(&[0])]),
        &bx("mdat", &[&payload(8, 1)]),
        &bx("meta", &[&[0; 4]]),
    ]);
    assert!(matches!(
        faststart::moov_first(&in_order),
        Some(Cow::Borrowed(same)) if std::ptr::eq(same, in_order.as_slice())
    ));

    // Nothing to move, or nothing to move it in front of, or two indexes.
    assert!(faststart::moov_first(&concat(&[&ftyp, &mdat])).is_none());
    assert!(faststart::moov_first(&concat(&[&ftyp, &moov])).is_none());
    assert!(faststart::moov_first(&concat(&[&ftyp, &mdat, &moov, &moov])).is_none());
    assert_eq!(faststart::is_moov_first(&concat(&[&ftyp, &mdat])), None);

    // Boxes that do not tile the file: one running past the end, a stub, a size smaller than its
    // header.
    let whole = concat(&[&ftyp, &mdat, &moov]);
    let truncated = &whole[..whole.len() - 5];
    assert!(faststart::moov_first(truncated).is_none());
    assert_eq!(faststart::is_moov_first(truncated), None);
    assert!(faststart::moov_first(&concat(&[&ftyp, &mdat, &moov, &[0; 3]])).is_none());
    let mut tiny = mdat.clone();
    tiny[..4].copy_from_slice(&4u32.to_be_bytes());
    assert!(faststart::moov_first(&concat(&[&ftyp, &tiny, &moov])).is_none());

    // A chunk table claiming more entries than it holds.
    let mut lying = stco(&[data]);
    lying[12..16].copy_from_slice(&1000u32.to_be_bytes());
    assert!(faststart::moov_first(&concat(&[&ftyp, &mdat, &moov_box(&[lying])])).is_none());
}
