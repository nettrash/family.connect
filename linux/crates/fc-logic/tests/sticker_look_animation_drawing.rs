//! `AnimationDrawingTests`, the one test whose subject is the sticker shelf: a pack item that was
//! removed leaves it (issue #96). The file's other six tests — the chat-list row, the quote, the
//! viewer's title, the tile's autoplay, the send's flag and the conversation's queue — are about
//! `ChatListModel`, `Quotes`, `MediaAlbum`, `AnimationPrep`, `AnimationSend`/`ComposerStaging` and
//! `ConversationModel`, each in its own module's `*_animation_drawing.rs` (the chat-list row in
//! `chat_list_media_rows.rs`; the send's flag and its `ComposerStaging` label in
//! `media_outbox_animation_drawing.rs`).

use std::cell::RefCell;
use std::rc::Rc;

use fc_logic::sticker_look::StickerShelf;

/// A picture as the window holds one: a thing with an identity, compared by reference (the C#'s
/// `HashSet<Picture>` of a class without its own equality).
struct Picture {
    name: &'static str,
}

#[test]
fn a_removed_sticker_leaves_the_shelf_and_is_let_go_unless_shown() {
    let shown: Rc<RefCell<Vec<Rc<Picture>>>> = Rc::default();
    let let_go: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let mut shelf = {
        let shown = Rc::clone(&shown);
        let let_go = Rc::clone(&let_go);
        StickerShelf::new(1000, 10)
            .with_showing(move |picture: &Rc<Picture>| {
                shown.borrow().iter().any(|held| Rc::ptr_eq(held, picture))
            })
            .with_let_go(move |picture: &Rc<Picture>| let_go.borrow_mut().push(picture.name))
    };
    let a = Rc::new(Picture { name: "a" });
    let b = Rc::new(Picture { name: "b" });
    shelf.put(1, Some(Rc::clone(&a)), 300);
    shelf.put(!5_i64, Some(Rc::clone(&b)), 200);
    shown.borrow_mut().push(Rc::clone(&b));
    assert!(shelf.remove(1));
    assert_eq!(vec!["a"], *let_go.borrow());
    assert_eq!(200, shelf.held());
    assert!(shelf.try_get(1).is_none());
    // One on screen leaves the shelf without being let go under its image.
    assert!(shelf.remove(!5_i64));
    assert_eq!(vec!["a"], *let_go.borrow());
    assert_eq!(0, shelf.held());
    assert!(!shelf.remove(1));
    assert_eq!(0, shelf.count());
}
