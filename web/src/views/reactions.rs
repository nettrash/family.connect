//! Reactions: the chips under a bubble, "See who reacted", and the full
//! "More reactions…" catalogue — drawn from the rules ported from the Apple
//! app (fc_text::reactions, fc_text::emoji), so a message's reactions read
//! the same everywhere.

use std::collections::{HashMap, HashSet};

use fc_text::emoji::EMOJI_CATALOG;
pub use fc_text::emoji::QUICK_REACTIONS;
use fc_text::reactions::{self as rules, ReactionDetail};
use yew::prelude::*;

use crate::model::Reaction;

/// One chip under a bubble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub emoji: String,
    pub count: usize,
    pub mine: bool,
}

fn to_rules(reactions: &[Reaction]) -> Vec<rules::Reaction> {
    reactions
        .iter()
        .map(|reaction| rules::Reaction::new(reaction.user_id, &reaction.emoji))
        .collect()
}

/// The chips, in the order each emoji was first chosen — not by popularity,
/// which would reshuffle them under the reader — each counting everybody,
/// a blocked member included.
pub fn chips(reactions: &[Reaction], me: i64) -> Vec<Chip> {
    rules::reaction_chips(&to_rules(reactions), me)
        .into_iter()
        .map(|chip| Chip {
            emoji: chip.emoji,
            count: chip.count,
            mine: chip.includes_me,
        })
        .collect()
}

/// The rows of "See who reacted": the chips' order, "You" first, and a
/// blocked reactor left out of the NAMES (the chip still counts them).
pub fn details(
    reactions: &[Reaction],
    names: &HashMap<i64, String>,
    me: i64,
    blocked: &HashSet<i64>,
) -> Vec<ReactionDetail> {
    rules::reaction_details(&to_rules(reactions), names, me, blocked)
}

#[derive(Properties, PartialEq)]
pub struct EmojiPickerProps {
    pub on_pick: Callback<String>,
    pub on_close: Callback<()>,
}

/// The whole catalogue, a category at a time.
#[function_component(EmojiPicker)]
pub fn emoji_picker(props: &EmojiPickerProps) -> Html {
    let category = use_state(|| 0usize);
    let close = {
        let on_close = props.on_close.clone();
        Callback::from(move |_: MouseEvent| on_close.emit(()))
    };
    let on_key = {
        let on_close = props.on_close.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                on_close.emit(());
            }
        })
    };
    let current = &EMOJI_CATALOG[(*category).min(EMOJI_CATALOG.len() - 1)];
    html! {
        <div class="dialog-backdrop" onkeydown={on_key}>
            <div class="dialog emoji-picker" role="dialog" aria-modal="true" aria-label="More reactions">
                <div class="emoji-tabs" role="tablist">
                    { for EMOJI_CATALOG.iter().enumerate().map(|(index, group)| {
                        let pick = {
                            let category = category.clone();
                            Callback::from(move |_: MouseEvent| category.set(index))
                        };
                        html! {
                            <button
                                role="tab"
                                class={classes!((index == *category).then_some("is-active"))}
                                aria-selected={(index == *category).to_string()}
                                title={group.name}
                                onclick={pick}
                            >
                                { group.emoji.first().copied().unwrap_or("·") }
                            </button>
                        }
                    }) }
                </div>
                <div class="emoji-grid" role="tabpanel" aria-label={current.name}>
                    { for current.emoji.iter().map(|emoji| {
                        let on_pick = props.on_pick.clone();
                        let chosen = emoji.to_string();
                        html! {
                            <button onclick={Callback::from(move |_: MouseEvent| on_pick.emit(chosen.clone()))}>
                                { *emoji }
                            </button>
                        }
                    }) }
                </div>
                <div class="dialog-actions">
                    <button class="secondary" onclick={close}>{ "Cancel" }</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn reaction(user_id: i64, emoji: &str) -> Reaction {
        Reaction {
            user_id,
            emoji: emoji.into(),
        }
    }

    /// The ported rules, through the web's own model type.
    #[wasm_bindgen_test]
    fn chips_keep_first_seen_order_count_everyone_and_mark_mine() {
        let list = vec![reaction(9, "👍"), reaction(7, "❤️"), reaction(11, "👍")];
        let chips = chips(&list, 7);
        assert_eq!(
            chips,
            vec![
                Chip {
                    emoji: "👍".into(),
                    count: 2,
                    mine: false
                },
                Chip {
                    emoji: "❤️".into(),
                    count: 1,
                    mine: true
                },
            ]
        );
        let names = HashMap::from([(9, "Anna".to_string())]);
        let rows = details(&list, &names, 7, &HashSet::from([11]));
        assert_eq!(
            rows[0].names,
            vec!["Anna".to_string()],
            "the blocked reactor is not named"
        );
        assert_eq!(rows[1].names, vec!["You".to_string()]);
    }
}
