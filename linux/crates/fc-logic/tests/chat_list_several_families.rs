//! `SeveralFamiliesTests.TheChatListIsTheFamilyShowns`, whose subject is `ChatListModel`: the chat
//! list is the family SHOWN's — its family chat pinned first, its assistant's chat, and the direct
//! chats it lists — none of another family's.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use fc_core::protocol::{ChatDto, ChatRowDto};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_logic::chat_list::ChatListModel;
use time::macros::datetime;

#[test]
fn the_chat_list_is_the_family_showns() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let shown = Rc::new(Cell::new(Some(3)));
    let reading = shown.clone();
    let chats = Rc::new(
        ChatStore::new(&cache)
            .with_me(|| 7)
            .with_current(move || reading.get()),
    );
    let chat = |id: i64, kind: &str, title: &str, families: Vec<i64>| {
        ChatRowDto::new(ChatDto {
            family_ids: Some(families),
            peer_user_id: (kind == "direct").then_some(9),
            ..ChatDto::new(id, kind, title)
        })
    };
    chats.replace(
        &[
            chat(42, "family", "The Smiths", vec![3]),
            chat(43, "family", "Ivanovs", vec![5]),
            chat(44, "direct", "Bob", vec![3, 5]),
            chat(45, "ai", "Assistant", vec![3]),
            chat(46, "ai", "Assistant", vec![5]),
        ],
        None,
    )?;
    let list = ChatListModel::new(chats, || 7, Rc::new(common::english()));
    let now = datetime!(2026-10-22 10:00 UTC);
    let sorted = |rows: &[fc_logic::chat_list::ChatRow]| {
        let mut ids: Vec<i64> = rows.iter().map(|row| row.chat.id).collect();
        ids.sort_unstable();
        ids
    };

    let rows = list.rows(now)?;
    assert_eq!(42, rows[0].chat.id);
    assert_eq!(vec![42, 44, 45], sorted(&rows));
    assert_eq!(5, list.held().len());

    shown.set(Some(5));
    let rows = list.rows(now)?;
    assert_eq!(43, rows[0].chat.id);
    assert_eq!(vec![43, 44, 46], sorted(&rows));
    Ok(())
}
