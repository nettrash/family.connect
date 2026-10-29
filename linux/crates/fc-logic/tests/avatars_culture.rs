//! `CultureTests.AnImageTypeIsStillAllowedOnATurkishComputer`, whose subject is `AvatarRules`. (The
//! file's other test is `ChatListModel.When`'s: `chat_list_culture.rs`.)

use fc_logic::avatars::avatar_rules;

/// THE TURKISH I: `"IMAGE/JPEG"` lower-cased in Turkish is `"ımage/jpeg"` with a dotless ı, so a
/// comparison through the reader's alphabet would refuse a perfectly good photograph. Rust has no ambient culture to set, so the
/// C#'s `CultureInfo.CurrentCulture = tr-TR` has nothing to stand for: the assertions are its own,
/// and one more pins that a dotless ı is not an `i` here either (it is not under .NET's
/// `OrdinalIgnoreCase`, measured).
#[test]
fn an_image_type_is_still_allowed_on_a_turkish_computer() {
    assert!(avatar_rules::is_allowed("IMAGE/JPEG"));
    assert!(avatar_rules::is_allowed("image/png"));
    assert!(!avatar_rules::is_allowed("image/heic"));
    assert!(!avatar_rules::is_allowed("\u{131}mage/png"));
}
