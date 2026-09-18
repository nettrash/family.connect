//! A person's picture, or their initials when they have none; the family's
//! house (ios Views/InitialsAvatar.swift).
//!
//! The initials draw at once and the picture replaces them when it lands, so
//! a row never waits on the network to render. A picture is fetched only for
//! a real `avatar_version` — 0 is "none" — and is cached under the version,
//! which the protocol never reuses for another picture.

use yew::prelude::*;

use crate::media::{use_media, Variant};

#[derive(Properties, PartialEq)]
pub struct AvatarProps {
    /// The name the initials come from.
    pub title: AttrValue,
    /// The family's circle: a house rather than initials.
    #[prop_or_default]
    pub family: bool,
    /// Whose picture — with `version` above 0, it replaces the initials.
    #[prop_or_default]
    pub user_id: Option<i64>,
    #[prop_or_default]
    pub version: i64,
    /// The circle's side, in CSS pixels.
    #[prop_or(40)]
    pub size: u32,
}

#[function_component(Avatar)]
pub fn avatar(props: &AvatarProps) -> Html {
    let wanted = props.user_id.is_some() && props.version > 0 && !props.family;
    let picture = use_media(
        props.user_id.unwrap_or_default(),
        Variant::Avatar(props.version),
        wanted,
    );
    let size = props.size;
    // Decorative beside the name it stands for, which is always drawn next to
    // it: a screen reader hearing "AS, Anna Smith" learns nothing twice.
    html! {
        <span
            class={classes!("avatar", props.family.then_some("is-family"))}
            style={format!("width:{size}px;height:{size}px;font-size:{:.1}px;", f64::from(size) * 0.36)}
            aria-hidden="true"
        >
            if let Some(url) = picture.filter(|_| wanted) {
                <img src={url} alt="" draggable="false" />
            } else if props.family {
                <svg viewBox="0 0 24 24" width={format!("{:.0}", f64::from(size) * 0.46)} height={format!("{:.0}", f64::from(size) * 0.46)}>
                    <path fill="currentColor" d="M12 2.7 1.8 11.6a.9.9 0 0 0 1.2 1.3L4 12v8.1c0 .9.7 1.6 1.6 1.6h4.2v-6h4.4v6h4.2c.9 0 1.6-.7 1.6-1.6V12l1 .9a.9.9 0 0 0 1.2-1.3Z" />
                </svg>
            } else {
                { fc_text::avatar::initials(&props.title) }
            }
        </span>
    }
}
