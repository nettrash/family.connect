//! The faces P1 draws — the chat list's rows, a sender above their bubbles, the composer's `@`
//! list: [`avatar`] is the ONE function that makes a face. It is `adw::Avatar` with the name's
//! initials, a house for the family chat and people for a group (the Mac's `InitialsAvatar`) — and
//! for a person, their picture over the initials once it lands (P2's `attachments::avatars`,
//! keyed by user AND the roster's avatar version, §8.3).

use fc_logic::groups::ChatFace;

/// A face of `size` px for `name`. `user_id`: the person it is, when it is one (a direct chat's
/// peer, a sender) — what P2's picture loader keys on with the roster's avatar version.
pub fn avatar(name: &str, face: ChatFace, user_id: Option<i64>, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(name), true);
    match face {
        ChatFace::House => {
            avatar.set_show_initials(false);
            avatar.set_icon_name(Some("user-home-symbolic"));
        }
        ChatFace::People => {
            avatar.set_show_initials(false);
            avatar.set_icon_name(Some("system-users-symbolic"));
        }
        ChatFace::Initials => {
            if let Some(user_id) = user_id {
                crate::attachments::avatars::show_member(&avatar, user_id);
            }
        }
    }
    avatar
}
