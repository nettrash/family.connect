//! `AnimationDrawingTests.AConversationQueuesAnAnimationWithItsFlag`: the flag is written with the
//! row and goes on the wire with the message — REST and socket alike read the row.

mod common;

use std::rc::Rc;

use fc_core::protocol::QueueOptions;
use fc_core::store::StoreResult;
use fc_logic::conversation::ConversationModel;

use common::chat::{CHAT, ChatRig, socket};
use common::server::{AnyPath, api_over};

#[test]
fn a_conversation_queues_an_animation_with_its_flag() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    // `Refusing`: every request answered 503, with nothing in it.
    let api = api_over(Rc::new(AnyPath::new(503, "")), &rig.clock);
    let silent = socket(false);
    let pipeline = rig.pipeline(&silent, &api);
    let chat = ConversationModel::new(
        CHAT,
        rig.chats.clone(),
        api,
        pipeline,
        Rc::new(silent),
        Rc::new(rig.clock.clone()),
    )?
    .with_outbox(rig.outbox.clone());

    let row = chat.send(
        "ha",
        &QueueOptions {
            pending_files: vec!["0123456789abcdef0123456789abcdef".to_string()],
            animation: true,
            ..QueueOptions::default()
        },
    )?;

    assert!(row.animation);
    assert!(!row.round);
    let all = rig.outbox.all()?;
    assert_eq!(1, all.len());
    assert!(all[0].animation);
    Ok(())
}
