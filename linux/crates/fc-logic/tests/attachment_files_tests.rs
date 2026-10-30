//! `AttachmentFilesTests`: the web client's attachment rules, with the web client's own vectors.

use fc_core::protocol::AttachmentDto;
use fc_logic::attachment_files::{self, TileSource};

#[test]
fn a_long_name_gives_up_its_middle() {
    assert_eq!(
        attachment_files::middle_truncate("short.pdf", 40),
        "short.pdf"
    );
    let cut =
        attachment_files::middle_truncate(&format!("{}.pdf", "Quarterly report ".repeat(5)), 20);
    assert_eq!(cut.chars().count(), 20);
    assert!(cut.starts_with("Quarterly re"));
    assert!(cut.ends_with("t .pdf"));
    assert!(cut.contains('…'));
    // Too little room to say anything useful: the name as it is.
    assert_eq!(attachment_files::middle_truncate("abcdefgh", 4), "abcdefgh");
}

/// Counted in scalars: a Rust string cannot hold half a surrogate pair at all, so what is asked is
/// that the cut is twelve whole characters and none of them a replacement.
#[test]
fn a_cut_never_splits_an_emoji() {
    let face = char::from_u32(0x1F600).unwrap();
    let name = format!("{}.png", face.to_string().repeat(30));
    let cut = attachment_files::middle_truncate(&name, 12);
    assert_eq!(cut.chars().count(), 12);
    assert!(cut.chars().all(|c| c != char::REPLACEMENT_CHARACTER));
    assert!(cut.starts_with(face));
    assert!(cut.ends_with("png"));
}

/// The name is the sender's, and the server only checks its length, so everything Windows refuses
/// has to be caught here.
#[test]
fn a_saved_name_is_never_a_path_and_never_a_character_windows_refuses() {
    assert_eq!(
        attachment_files::safe_file_name("receipts.pdf"),
        "receipts.pdf"
    );
    // A path is reduced to its last part, in either slash.
    assert_eq!(
        attachment_files::safe_file_name(r"C:\Users\nora\receipts.pdf"),
        "receipts.pdf"
    );
    assert_eq!(
        attachment_files::safe_file_name("../../etc/receipts.pdf"),
        "receipts.pdf"
    );
    // Trailing dots and spaces are not a name Windows will keep.
    assert_eq!(
        attachment_files::safe_file_name("  receipts.  "),
        "receipts"
    );
    // Nothing usable left: something has to be written somewhere.
    assert_eq!(attachment_files::safe_file_name("   "), "attachment.bin");
    assert_eq!(attachment_files::safe_file_name("/"), "attachment.bin");
}

/// Win32 resolves its device names in EVERY directory and with ANY extension, so an attachment
/// called "CON.txt" would be written to the console rather than to a file. Anyone in the family can
/// name a file that, so the name must stop being a device.
#[test]
fn a_device_name_is_not_left_as_one() {
    assert_eq!(attachment_files::safe_file_name("CON"), "_CON");
    assert_eq!(attachment_files::safe_file_name("con.txt"), "_con.txt");
    assert_eq!(attachment_files::safe_file_name("NUL.pdf"), "_NUL.pdf");
    assert_eq!(attachment_files::safe_file_name("COM1.jpg"), "_COM1.jpg");
    assert_eq!(attachment_files::safe_file_name("LPT9"), "_LPT9");
    assert_eq!(attachment_files::safe_file_name("PRN.doc"), "_PRN.doc");
    assert_eq!(attachment_files::safe_file_name("AUX."), "_AUX");
    // Not devices: only the exact stems are reserved.
    assert_eq!(
        attachment_files::safe_file_name("CONtract.pdf"),
        "CONtract.pdf"
    );
    assert_eq!(attachment_files::safe_file_name("COM10.txt"), "COM10.txt");
    assert_eq!(attachment_files::safe_file_name("nulls.csv"), "nulls.csv");
    assert_eq!(attachment_files::safe_file_name(".con"), ".con");
}

fn with(id: i64, kind: &str, mime: Option<&str>, name: Option<&str>) -> AttachmentDto {
    AttachmentDto {
        mime: mime.map(str::to_string),
        name: name.map(str::to_string),
        ..AttachmentDto::new(id, kind)
    }
}

#[test]
fn a_download_is_named_for_what_it_is() {
    assert_eq!(
        attachment_files::file_name(&with(9, "file", None, Some("receipts.pdf"))),
        "receipts.pdf"
    );
    assert_eq!(
        attachment_files::file_name(&with(34, "photo", Some("image/jpeg"), None)),
        "photo-34.jpg"
    );
    assert_eq!(
        attachment_files::file_name(&with(35, "video", Some("video/quicktime"), None)),
        "video-35.mov"
    );
    assert_eq!(
        attachment_files::file_name(&with(
            36,
            "audio",
            Some("audio/mp4; codecs=mp4a.40.2"),
            None
        )),
        "audio-36.m4a"
    );
    assert_eq!(
        attachment_files::file_name(&with(37, "file", None, Some(""))),
        "file-37.bin"
    );
}

/// Which bytes a tile asks for — never a whole video.
#[test]
fn a_tile_never_downloads_a_video_to_draw_itself() {
    let previewed = |kind: &str| AttachmentDto {
        has_preview: true,
        ..AttachmentDto::new(1, kind)
    };
    assert_eq!(
        attachment_files::source_for(&previewed("photo")),
        TileSource::Preview
    );
    assert_eq!(
        attachment_files::source_for(&AttachmentDto::new(1, "photo")),
        TileSource::Original
    );
    assert_eq!(
        attachment_files::source_for(&previewed("video")),
        TileSource::Preview
    );
    assert_eq!(
        attachment_files::source_for(&AttachmentDto::new(1, "video")),
        TileSource::None
    );
}

/// The port's own: what Windows refuses is replaced wherever it is, and Unicode white space is
/// trimmed as .NET trims it.
#[test]
fn every_refused_character_becomes_an_underscore() {
    assert_eq!(
        attachment_files::safe_file_name("a<b>c:d\"e|f?g*h\u{1}i.txt"),
        "a_b_c_d_e_f_g_h_i.txt"
    );
    assert_eq!(
        attachment_files::safe_file_name("\u{3000}name.txt\u{2003}"),
        "name.txt"
    );
    assert_eq!(attachment_files::safe_file_name("Привет.pdf"), "Привет.pdf");
}
