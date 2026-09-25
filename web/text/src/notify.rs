//! What a browser's own notification says (docs/protocol.md, "A browser is
//! a client too", and the push rules' "Titles").
//!
//! The title is the push's title, because a family that uses a phone and a
//! tab should read the same words in both places. The BODY is the one a
//! server with `include_message_body = false` would send: a browser cannot
//! know whether the operator wanted family text on a lock screen, so it
//! never puts it there.
use crate::i18n::{t, t2};

/// The body of a message notification — never the message. A function, not
/// a const: a translated string is not a constant.
pub fn new_message() -> &'static str {
    t("New message")
}

/// The body of a board-note notification.
pub fn new_note() -> &'static str {
    t("New note")
}

/// Who a notification is from.
///
/// A direct chat is the sender alone; the family chat names the family
/// first, because a phone's lock screen shows a list and "Anna" alone does
/// not say where she said it. A message that names the reader says so in
/// the title — the same notification, a different title, never a second
/// one.
pub fn title(family: Option<&str>, sender: &str, mentioned: bool) -> String {
    match family {
        None => sender.to_string(),
        Some(family) if mentioned => t2("%@ — %@ mentioned you", family, sender),
        Some(family) => t2("%@ — %@", family, sender),
    }
}

/// The count in the page's title: "(3) Family Connect", and the bare name
/// at zero. A hundred and more is drawn as "99+" — the exact number stops
/// being the point, and a tab strip has no room for it.
pub fn page_title(name: &str, unread: i64) -> String {
    match unread {
        count if count <= 0 => name.to_string(),
        count if count > 99 => format!("(99+) {name}"),
        count => format!("({count}) {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_chat_is_the_sender_and_the_family_chat_says_where() {
        assert_eq!(title(None, "Anna", false), "Anna");
        assert_eq!(
            title(Some("The Smiths"), "Anna", false),
            "The Smiths — Anna"
        );
        assert_eq!(
            title(Some("The Smiths"), "Anna", true),
            "The Smiths — Anna mentioned you"
        );
        // A mention in a DIRECT chat adds nothing: it is already from them,
        // to the reader alone.
        assert_eq!(title(None, "Anna", true), "Anna");
    }

    #[test]
    fn the_page_title_carries_the_count() {
        assert_eq!(page_title("Family Connect", 0), "Family Connect");
        assert_eq!(page_title("Family Connect", -1), "Family Connect");
        assert_eq!(page_title("Family Connect", 3), "(3) Family Connect");
        assert_eq!(page_title("Family Connect", 99), "(99) Family Connect");
        assert_eq!(page_title("Family Connect", 100), "(99+) Family Connect");
    }
}
