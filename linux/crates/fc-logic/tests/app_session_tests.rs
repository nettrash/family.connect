//! `AppSessionTests`: which screen the app is on, and the three ways a signed-in state ends.

mod common;

use std::cell::{Cell, RefCell};
use std::pin::pin;
use std::rc::Rc;

use fc_core::protocol::{
    ChatDto, ChatRowDto, MemoryTokenStore, RoundVideoLimits, TokenStore, error_codes,
};
use fc_core::rt::Cancel;
use fc_core::store::{ChatStore, Database, OutboxRow, OutboxStore, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::app_session::{AppSession, Gate, SessionEnd};
use fc_logic::assistant_consent;
use fc_logic::join_memory::{AwaitingJoin, MemoryAwaitingJoin};
use time::macros::datetime;

use common::server::Server;
use common::session::api_with;

const TOKEN: &str =
    r#"{"token": "t0ken", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#;

fn me(extra: &str) -> String {
    format!(
        r#"{{"user": {{"id": 7, "username": "anna", "display_name": "Anna"}},
         "blocked_user_ids": [], "calls_enabled": true, "max_family_members": 8,
         "support_contact": "help@example.com"{extra}}}"#
    )
}

const IN_FAMILY: &str =
    r#", "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "owner" "#;

const A_MEMBER: &str =
    r#", "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "member" "#;

const WAITING: &str = r#", "pending_join_request": {"family_id": 3, "family_name": "The Smiths"} "#;

struct Rig {
    session: AppSession,
    tokens: Rc<MemoryTokenStore>,
    server: Server,
    clock: FakeClock,
    cache: Rc<Database>,
}

impl Rig {
    fn run<F: std::future::Future>(&self, future: F) -> F::Output {
        drive(&self.clock, future)
    }

    fn sign_in(
        &self,
        username: &str,
        password: &str,
    ) -> StoreResult<Option<fc_core::protocol::ApiError>> {
        self.run(self.session.sign_in(username, password))
    }

    fn refresh(&self) -> StoreResult<Option<fc_core::protocol::ApiError>> {
        self.run(self.session.refresh())
    }

    fn endings(&self) -> Rc<RefCell<Vec<SessionEnd>>> {
        let endings = Rc::new(RefCell::new(Vec::new()));
        let heard = endings.clone();
        self.session
            .on_ended()
            .subscribe(move |end| heard.borrow_mut().push(*end));
        endings
    }

    fn token(&self) -> Option<String> {
        self.tokens.token()
    }
}

fn build_over(server: &Server, token: Option<&str>, cache: &Rc<Database>) -> Rig {
    let clock = FakeClock::at(datetime!(2026-09-12 10:00 UTC));
    let tokens = Rc::new(MemoryTokenStore::new(token));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    Rig {
        session: AppSession::new(api, tokens.clone(), cache.clone()),
        tokens,
        server: server.clone(),
        clock,
        cache: cache.clone(),
    }
}

fn build(server: &Server, token: Option<&str>) -> StoreResult<Rig> {
    Ok(build_over(server, token, &Database::open_in_memory()?))
}

fn with_memory(
    server: &Server,
    memory: Rc<dyn AwaitingJoin>,
    token: Option<&str>,
) -> StoreResult<Rig> {
    let clock = FakeClock::at(datetime!(2026-09-12 10:00 UTC));
    let cache = Database::open_in_memory()?;
    let tokens = Rc::new(MemoryTokenStore::new(token));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    Ok(Rig {
        session: AppSession::new(api, tokens.clone(), cache.clone()).with_awaiting_join(memory),
        tokens,
        server: server.clone(),
        clock,
        cache,
    })
}

const INVALID_CREDENTIALS: &str = r#"{"error": {"code": "invalid_credentials", "message": "no"}}"#;

#[test]
fn signing_in_puts_the_app_on_the_screen_me_answers_for() -> StoreResult<()> {
    let server = Server::new();
    server.on("/auth/login", TOKEN).on("/me", &me(IN_FAMILY));
    let rig = build(&server, None)?;
    let screens = Rc::new(RefCell::new(Vec::new()));
    let seen = screens.clone();
    rig.session
        .on_changed()
        .subscribe(move |state| seen.borrow_mut().push(state.gate));

    assert_eq!(None, rig.sign_in("anna", "hunter2")?);

    assert_eq!(Some("t0ken".to_string()), rig.token());
    let state = rig.session.state();
    assert_eq!(Gate::Owner, state.gate);
    assert!(state.is_owner());
    assert!(state.can_chat());
    assert_eq!("The Smiths", state.family.as_ref().expect("a family").name);
    // The capabilities arrive with the answer rather than as errors later.
    assert!(state.calls_enabled);
    assert_eq!(8, state.max_family_members);
    assert_eq!(vec![Gate::Owner], *screens.borrow());
    Ok(())
}

/// A `GET /me` REFRESH KEEPS THE FAMILY'S ASSISTANT. Only `GET /families/mine` carries it, so a
/// refresh that rebuilt the snapshot without it left the app with no assistant at all — no
/// Assistant card in Settings, and no consent line over the composer, which is the one place a
/// member can agree that their words may go to the model (docs/protocol.md, "Consenting to the
/// assistant"). It is worst right after they agree, because agreeing is followed by exactly this
/// refresh.
#[test]
fn a_refresh_keeps_the_assistant_it_was_not_told_about() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                 "members": [{"user_id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "@ai",
                               "processor": "Microsoft — Azure OpenAI (Sweden Central)"}}"#,
        );
    let rig = build(&server, None)?;
    assert_eq!(None, rig.sign_in("anna", "hunter2")?);
    assert_eq!(None, rig.run(rig.session.refresh_family())?);
    let processor = |rig: &Rig| {
        rig.session
            .state()
            .assistant
            .and_then(|assistant| assistant.processor)
    };
    assert_eq!(
        Some("Microsoft \u{2014} Azure OpenAI (Sweden Central)".to_string()),
        processor(&rig)
    );

    // The refresh that follows agreeing, changing a name, or a reconnect.
    assert_eq!(None, rig.refresh()?);

    assert_eq!(
        Some("Microsoft \u{2014} Azure OpenAI (Sweden Central)".to_string()),
        processor(&rig)
    );
    assert!(assistant_consent::is_available(processor(&rig).as_deref()));
    Ok(())
}

/// The video message's limits come with the family's own document, as the assistant does — and a
/// `GET /me` refresh keeps them, because only that document carries them (docs/protocol.md, "Video
/// messages").
#[test]
fn a_refresh_keeps_the_video_messages_limits() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths"},
                 "members": [{"user_id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "max_round_video_ms": 60000, "max_round_video_bytes": 4194304}"#,
        );
    let rig = build(&server, None)?;
    assert_eq!(None, rig.sign_in("anna", "hunter2")?);
    assert_eq!(None, rig.session.state().round_video);
    assert_eq!(None, rig.run(rig.session.refresh_family())?);
    let limits = RoundVideoLimits {
        max_ms: 60_000,
        max_bytes: 4_194_304,
    };
    assert_eq!(Some(limits), rig.session.state().round_video);

    assert_eq!(None, rig.refresh()?);

    assert_eq!(Some(limits), rig.session.state().round_video);
    // Applied again unchanged: nothing is published.
    let published = Rc::new(Cell::new(0));
    let counted = published.clone();
    rig.session
        .on_changed()
        .subscribe(move |_| counted.set(counted.get() + 1));
    rig.session.apply_round_video(Some(limits));
    assert_eq!(0, published.get());
    rig.session.apply_round_video(None);
    assert_eq!(1, published.get());
    assert_eq!(None, rig.session.state().round_video);
    Ok(())
}

/// And they go with the family: a member with none can record for nobody.
#[test]
fn the_video_messages_limits_go_with_the_family() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .then("/me", &[(200, Some(&me(IN_FAMILY))), (200, Some(&me("")))])
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths"},
                 "members": [{"user_id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "max_round_video_ms": 60000, "max_round_video_bytes": 4194304}"#,
        );
    let rig = build(&server, None)?;
    assert_eq!(None, rig.sign_in("anna", "hunter2")?);
    assert_eq!(None, rig.run(rig.session.refresh_family())?);
    assert!(rig.session.state().round_video.is_some());

    assert_eq!(None, rig.refresh()?);

    assert_eq!(None, rig.session.state().round_video);
    // And nothing applies them to a session with no family.
    rig.session.apply_round_video(Some(RoundVideoLimits {
        max_ms: 60_000,
        max_bytes: 1,
    }));
    assert_eq!(None, rig.session.state().round_video);
    Ok(())
}

/// But it goes with the family: an assistant belongs to one.
#[test]
fn the_assistant_goes_when_the_family_does() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        // In the family when they sign in, out of it by the next refresh.
        .then("/me", &[(200, Some(&me(IN_FAMILY))), (200, Some(&me("")))])
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths"},
                 "members": [{"user_id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "@ai", "processor": "Azure OpenAI"}}"#,
        );
    let rig = build(&server, None)?;
    assert_eq!(None, rig.sign_in("anna", "hunter2")?);
    assert_eq!(None, rig.run(rig.session.refresh_family())?);
    assert!(rig.session.state().assistant.is_some());

    // Removed from the family: the next /me says so, and the assistant is not theirs to keep.
    assert_eq!(None, rig.refresh()?);

    assert_eq!(Gate::NoFamily, rig.session.state().gate);
    assert_eq!(None, rig.session.state().assistant);
    Ok(())
}

#[test]
fn a_member_is_not_an_owner_and_an_account_with_no_family_is_at_the_gate() -> StoreResult<()> {
    let member = Server::new();
    member.on("/auth/login", TOKEN).on("/me", &me(A_MEMBER));
    let session = build(&member, None)?;
    session.sign_in("anna", "hunter2")?;
    assert_eq!(Gate::Member, session.session.state().gate);
    assert!(!session.session.state().is_owner());

    let alone = Server::new();
    alone.on("/auth/login", TOKEN).on("/me", &me(""));
    let gate = build(&alone, None)?;
    gate.sign_in("anna", "hunter2")?;
    assert_eq!(Gate::NoFamily, gate.session.state().gate);
    assert!(!gate.session.state().can_chat());

    let waiting = Server::new();
    waiting.on("/auth/login", TOKEN).on("/me", &me(WAITING));
    let pending = build(&waiting, None)?;
    pending.sign_in("anna", "hunter2")?;
    assert_eq!(Gate::Pending, pending.session.state().gate);
    assert_eq!(
        Some("The Smiths".to_string()),
        pending.session.state().pending_family_name
    );
    Ok(())
}

/// A WRONG PASSWORD IS NOT AN EXPIRED SESSION. Both are 401, and one of this app's own clients read
/// every one of them as "your session expired" — so a mistyped password told the user they had been
/// signed out of a session they were never in.
#[test]
fn a_wrong_password_is_answer_to_the_form_and_not_the_end_of_anything() -> StoreResult<()> {
    let server = Server::new();
    server.on_status("/auth/login", 401, Some(INVALID_CREDENTIALS));
    let rig = build(&server, None)?;
    let endings = rig.endings();

    let refused = rig.sign_in("anna", "wrong")?.expect("refused");

    assert_eq!(error_codes::INVALID_CREDENTIALS, refused.code);
    assert!(!refused.session_gone());
    assert!(endings.borrow().is_empty());
    assert_eq!(None, rig.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    Ok(())
}

/// And a failed sign-in never ends a session that IS open: the answer belongs to the form, and the
/// account already signed in on this device is none of its business.
#[test]
fn a_failed_sign_in_leaves_a_session_that_is_already_open_alone() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/me", &me(IN_FAMILY))
        .on_status("/auth/login", 401, Some(INVALID_CREDENTIALS));
    let rig = build(&server, Some("t0ken"))?;
    rig.refresh()?;
    let endings = rig.endings();

    assert_eq!(
        error_codes::INVALID_CREDENTIALS,
        rig.sign_in("bob", "wrong")?.expect("refused").code
    );

    assert!(endings.borrow().is_empty());
    assert_eq!(Some("t0ken".to_string()), rig.token());
    assert_eq!(Gate::Owner, rig.session.state().gate);
    Ok(())
}

/// THE SESSION ENDS ONCE. A dead session is discovered by whatever asks next, and on a reconnect
/// loop that is every few seconds — the user needs telling once.
#[test]
fn a_session_that_is_gone_ends_once_however_often_it_is_discovered() -> StoreResult<()> {
    let rig = build(&Server::new(), Some("t0ken"))?;
    let endings = rig.endings();

    rig.session.session_gone()?;
    rig.session.session_gone()?;
    rig.session.session_gone()?;

    assert_eq!(vec![SessionEnd::Expired], *endings.borrow());
    assert_eq!(None, rig.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    Ok(())
}

/// A FLAKY NETWORK IS NOT A SIGN-OUT. The alternative is an app that drops to the sign-in screen in
/// a lift — and asks for a password it has no business asking for.
#[test]
fn a_transient_failure_leaves_the_app_exactly_where_it_is() -> StoreResult<()> {
    let server = Server::new();
    server.on("/auth/login", TOKEN).then(
        "/me",
        &[(200, Some(&me(IN_FAMILY))), (502, Some("<html>502</html>"))],
    );
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;

    let failed = rig.refresh()?.expect("failed");

    assert!(failed.transient());
    assert_eq!(Gate::Owner, rig.session.state().gate);
    assert_eq!(Some("t0ken".to_string()), rig.token());
    Ok(())
}

/// A 401 on an authenticated read IS the session going away underneath them, and it lands in the
/// same place the socket's 4401 does.
#[test]
fn a401_on_an_authenticated_read_ends_the_session() -> StoreResult<()> {
    let server = Server::new();
    server.on("/auth/login", TOKEN).then(
        "/me",
        &[
            (200, Some(&me(IN_FAMILY))),
            (
                401,
                Some(r#"{"error": {"code": "unauthorized", "message": "gone"}}"#),
            ),
        ],
    );
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let endings = rig.endings();

    rig.refresh()?;

    assert_eq!(vec![SessionEnd::Expired], *endings.borrow());
    assert_eq!(None, rig.token());
    Ok(())
}

/// Being removed from a family is found out on `GET /me` and nowhere else: a removal this device
/// slept through raises no frame it will ever see.
#[test]
fn being_removed_from_a_family_is_found_out_on_the_next_read_and_takes_its_data_with_it()
-> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .then("/me", &[(200, Some(&me(A_MEMBER))), (200, Some(&me("")))]);
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let chats = ChatStore::new(&rig.cache);
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let endings = rig.endings();

    rig.refresh()?;

    assert_eq!(vec![SessionEnd::RemovedFromFamily], *endings.borrow());
    assert_eq!(Gate::NoFamily, rig.session.state().gate);
    // Removed is not refused: nobody asked to join anything.
    assert!(!rig.session.state().join_declined);
    // Their family's chats are not theirs to keep — but they are still signed in.
    assert!(chats.chats()?.is_empty());
    assert_eq!(Some("t0ken".to_string()), rig.token());
    Ok(())
}

#[test]
fn a_join_request_that_is_no_longer_pending_was_refused() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .then("/me", &[(200, Some(&me(WAITING))), (200, Some(&me("")))]);
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let endings = rig.endings();

    rig.refresh()?;

    assert_eq!(vec![SessionEnd::JoinRequestRejected], *endings.borrow());
    assert_eq!(Gate::NoFamily, rig.session.state().gate);
    Ok(())
}

/// A refusal is never said: the request just vanishes from `/me`. The door says so — on the next
/// read too — until a join or a family replaces it (ios AppSession.joinDeclined, web gate).
#[test]
fn a_declined_join_is_said_on_the_door_until_something_replaces_it() -> StoreResult<()> {
    let memory = Rc::new(MemoryAwaitingJoin::new());
    let server = Server::new();
    server.on("/auth/login", TOKEN).then(
        "/me",
        &[
            (200, Some(&me(WAITING))),
            (200, Some(&me(""))),
            (200, Some(&me(""))),
            (200, Some(&me(WAITING))),
            (200, Some(&me(""))),
            (200, Some(&me(A_MEMBER))),
        ],
    );
    let rig = with_memory(&server, memory.clone(), None)?;
    let seen = |rig: &Rig| {
        let state = rig.session.state();
        (state.gate, state.join_declined)
    };

    rig.sign_in("anna", "hunter2")?;
    assert_eq!((Gate::Pending, false, Some(7)), {
        let (gate, declined) = seen(&rig);
        (gate, declined, memory.user_id())
    });

    rig.refresh()?;
    assert_eq!((Gate::NoFamily, true, None), {
        let (gate, declined) = seen(&rig);
        (gate, declined, memory.user_id())
    });

    rig.refresh()?;
    assert!(
        rig.session.state().join_declined,
        "still at the door, still said"
    );

    rig.refresh()?;
    assert_eq!((Gate::Pending, false), seen(&rig));

    rig.refresh()?;
    assert!(rig.session.state().join_declined);
    rig.refresh()?;
    assert_eq!((Gate::Member, false), seen(&rig));
    Ok(())
}

/// A relaunch remembers the wait; an account that never waited — or a different one — is never told
/// it was declined.
#[test]
fn only_the_account_that_waited_is_told_after_a_relaunch() -> StoreResult<()> {
    let waited = Rc::new(MemoryAwaitingJoin::new());
    waited.set_user_id(Some(7));
    let server = Server::new();
    server.on("/me", &me(""));
    let relaunched = with_memory(&server, waited.clone(), Some("t0ken"))?;
    let endings = relaunched.endings();
    relaunched.refresh()?;
    assert!(relaunched.session.state().join_declined);
    assert_eq!(vec![SessionEnd::JoinRequestRejected], *endings.borrow());
    assert_eq!(None, waited.user_id());

    let someone = Rc::new(MemoryAwaitingJoin::new());
    someone.set_user_id(Some(8));
    let someone_else = with_memory(&server, someone, Some("t0ken"))?;
    someone_else.refresh()?;
    assert!(!someone_else.session.state().join_declined);

    let never_asked = with_memory(&server, Rc::new(MemoryAwaitingJoin::new()), Some("t0ken"))?;
    never_asked.refresh()?;
    let state = never_asked.session.state();
    assert_eq!((Gate::NoFamily, false), (state.gate, state.join_declined));
    Ok(())
}

/// Signing out tells the server — which is what stops this device's notifications — and being
/// unable to tell it changes nothing: the token is this device's to forget.
#[test]
fn signing_out_tells_the_server_and_forgets_everything_either_way() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on_status("/auth/logout", 204, None);
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let chats = ChatStore::new(&rig.cache);
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    OutboxStore::new(&rig.cache).queue(&OutboxRow::new("k", 42, "Dinner at 7?"))?;
    let endings = rig.endings();

    rig.run(rig.session.sign_out())?;

    assert!(
        rig.server
            .asked()
            .contains(&"/api/v1/auth/logout".to_string())
    );
    assert_eq!(vec![SessionEnd::SignedOut], *endings.borrow());
    assert_eq!(None, rig.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    assert!(chats.chats()?.is_empty());
    // THE OUTBOX GOES TOO: a queued message belongs to the account that wrote it, and a send that
    // landed in the next person's family would be the worst bug this app could have.
    assert!(OutboxStore::new(&rig.cache).all()?.is_empty());
    // The support address outlives the session: it is the server's, not the account's, and it is
    // what a sign-in screen shows when somebody cannot get in.
    assert_eq!(
        Some("help@example.com".to_string()),
        rig.session.state().support_contact
    );
    Ok(())
}

/// Not only a refusal: a request that cannot even be made — no route, a dead handler, a cancelled
/// token — must not leave the app signed in to something it has already forgotten how to talk to.
#[test]
fn a_sign_out_whose_request_cannot_even_be_made_is_still_a_sign_out() -> StoreResult<()> {
    // No /auth/logout route at all: the transport fails rather than answering.
    let server = Server::new();
    server.on("/auth/login", TOKEN).on("/me", &me(IN_FAMILY));
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let endings = rig.endings();

    rig.run(rig.session.sign_out())?;

    assert_eq!(vec![SessionEnd::SignedOut], *endings.borrow());
    assert_eq!(None, rig.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    Ok(())
}

#[test]
fn a_sign_out_the_network_refuses_is_still_a_sign_out() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on_status("/auth/logout", 502, Some("<html>502</html>"));
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;

    rig.run(rig.session.sign_out())?;

    assert_eq!(None, rig.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    Ok(())
}

/// Signing in as somebody else on a device that was signed in before must not leave the last
/// family's messages on screen — nor their outbox anywhere near this account's socket.
#[test]
fn a_new_sign_in_does_not_inherit_the_last_accounts_cache() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let chats = ChatStore::new(&cache);
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(
            42,
            "family",
            "The Other Family",
        ))],
        None,
    )?;
    let server = Server::new();
    server.on("/auth/login", TOKEN).on("/me", &me(IN_FAMILY));
    let rig = build_over(&server, None, &cache);

    rig.sign_in("bob", "hunter2")?;

    assert!(chats.chats()?.is_empty());
    Ok(())
}

/// A WRONG PASSWORD ON A PASSWORD CHANGE IS NOT AN EXPIRED SESSION EITHER. It is the same 401 and
/// the same distinction: the account is still there, and so is the session.
#[test]
fn a_wrong_current_password_does_not_sign_anybody_out() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on_status("/me/password", 401, Some(INVALID_CREDENTIALS));
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let endings = rig.endings();

    let refused = rig
        .run(rig.session.change_password("wrong", "hunter22"))
        .expect("refused");

    assert_eq!(error_codes::INVALID_CREDENTIALS, refused.code);
    assert!(endings.borrow().is_empty());
    assert_eq!(Some("t0ken".to_string()), rig.token());
    assert_eq!(Gate::Owner, rig.session.state().gate);
    Ok(())
}

/// Deleting the account ends everything — and it is its OWN ending, because there is nothing left
/// to sign back into and a screen offering to would be cruel as well as wrong.
#[test]
fn deleting_the_account_is_its_own_ending_and_takes_everything_with_it() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on_status("/me/delete", 204, None);
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let chats = ChatStore::new(&rig.cache);
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let endings = rig.endings();

    assert_eq!(None, rig.run(rig.session.delete_account("hunter2"))?);

    assert_eq!(vec![SessionEnd::AccountDeleted], *endings.borrow());
    assert_eq!(None, rig.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    assert!(chats.chats()?.is_empty());
    Ok(())
}

#[test]
fn a_refused_deletion_changes_nothing() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on_status("/me/delete", 401, Some(INVALID_CREDENTIALS));
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;
    let endings = rig.endings();

    let refused = rig
        .run(rig.session.delete_account("wrong"))?
        .expect("refused");

    assert_eq!(error_codes::INVALID_CREDENTIALS, refused.code);
    assert!(endings.borrow().is_empty());
    assert_eq!(Some("t0ken".to_string()), rig.token());
    assert_eq!(Gate::Owner, rig.session.state().gate);
    Ok(())
}

/// A family read that was IN FLIGHT when the gate moved publishes nothing: handing the window a
/// signed-out state carrying a family is how a sign-out draws a chat list for a second.
#[test]
fn a_family_read_that_lands_after_a_sign_out_publishes_nothing() -> StoreResult<()> {
    let held = Cancel::new();
    let release = held.clone();
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on_status("/auth/logout", 204, None)
        .on_async("/families/mine", move || {
            let held = held.clone();
            Box::pin(async move {
                held.cancelled().await;
                Ok((
                    200,
                    Some(
                        r#"{"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                             "members": [], "blocked_user_ids": []}"#
                            .to_string(),
                    ),
                ))
            })
        });
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;

    rig.run(async {
        // The family read is in flight when the user signs out.
        let mut reading = pin!(rig.session.refresh_family());
        assert!(futures::poll!(reading.as_mut()).is_pending());
        rig.session.sign_out().await?;
        release.cancel();
        assert_eq!(None, reading.await?);
        StoreResult::Ok(())
    })?;

    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    assert_eq!(None, rig.session.state().family);
    Ok(())
}

#[test]
fn the_familys_own_document_fills_in_the_assistant_and_the_roster() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", &me(IN_FAMILY))
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_history": true, "ai_vision": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "blocked_user_ids": [], "max_board_seq": 11,
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                               "draw": "/draw", "vision": true, "images": true}}"#,
        );
    let rig = build(&server, None)?;
    rig.sign_in("anna", "hunter2")?;

    assert_eq!(None, rig.run(rig.session.refresh_family())?);

    let state = rig.session.state();
    let assistant = state.assistant.as_ref().expect("an assistant");
    assert_eq!("Assistant", assistant.display_name);
    assert!(assistant.images);
    assert!(state.family.as_ref().expect("a family").ai_vision);
    // And the gate did not move: the family document says nothing about who is signed in.
    assert_eq!(Gate::Owner, state.gate);
    Ok(())
}

/// AN OFFLINE LAUNCH OPENS WHAT IT HOLDS (the Mac's `openOffline`): a token, chats in the cache and
/// the last `/me` this account was answered — the chats open from it, as they were, and the error
/// is still answered so the shell knows it is offline. Without chats there is nothing to open, and
/// the offline page stands; a session already on screen is left exactly where it is.
#[test]
fn an_offline_launch_opens_the_cached_chats_from_the_last_me() -> StoreResult<()> {
    let online = Server::new();
    online.on("/me", &me(IN_FAMILY));
    let cache = Database::open_in_memory()?;
    let first = build_over(&online, Some("t0ken"), &cache);
    first.refresh()?;
    assert_eq!(Gate::Owner, first.session.state().gate);

    // The next launch, with no network — and no chats yet: nothing to open.
    let offline = Server::new();
    offline.on_status("/me", 502, Some("<html>502</html>"));
    let empty = build_over(&offline, Some("t0ken"), &cache);
    assert!(empty.refresh()?.is_some());
    assert_eq!(Gate::SignedOut, empty.session.state().gate);

    // With the family's chat held, it opens — as the last answer had it.
    ChatStore::new(&cache).replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let relaunched = build_over(&offline, Some("t0ken"), &cache);
    let failed = relaunched.refresh()?.expect("still offline");
    assert!(failed.transient());
    let state = relaunched.session.state();
    assert_eq!(Gate::Owner, state.gate);
    assert_eq!(Some(7), state.me.map(|me| me.id));
    assert_eq!(Some("t0ken".to_string()), relaunched.token());
    Ok(())
}
