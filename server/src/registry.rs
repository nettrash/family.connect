//! In-process WebSocket connection registry.
//!
//! Maps user id -> live connections so REST handlers and WS frame handlers
//! can fan frames out without touching sockets directly. Design choices:
//!
//! - Each connection owns a **bounded** mpsc queue. Fan-out uses `try_send`,
//!   never an await: a slow consumer must not stall delivery to everyone
//!   else. When a queue is full the connection is *kicked* instead — the
//!   socket is a live wire, not a queue; the client resyncs over REST.
//! - Kick/close requests travel over a per-connection `watch` channel
//!   carrying the WebSocket close code. A watch write never blocks and
//!   works even when the frame queue is the very thing that's full, which
//!   is why it exists as a separate channel rather than a `Close` sentinel
//!   frame in the mpsc.
//! - The user map is behind an async `RwLock`; senders are cloned under the
//!   read lock and `try_send` happens after it is released, so delivery
//!   work never holds the lock.
//! - The registry also owns the process-wide shutdown `CancellationToken`:
//!   connection tasks are spawned by axum, out of main's reach, so the
//!   token has to live somewhere both main and the tasks can see.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{RwLock, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::ws::ServerFrame;

/// Close code for "the server is going away / this socket is being dropped"
/// (normal WebSocket 1001).
pub const CLOSE_GOING_AWAY: u16 = 1001;

/// Application close code for "your session no longer exists" — protocol.md
/// reserves 4401; the client wipes local state and returns to login.
pub const CLOSE_SESSION_GONE: u16 = 4401;

/// One live connection as seen by senders.
#[derive(Clone)]
struct ConnHandle {
    conn_id: u64,
    session_id: i64,
    /// The client said `families=1` on the upgrade: it knows about several
    /// families and may be sent frames about every one of them. A
    /// connection without it is an app from before 2026-10-22 and hears
    /// about its account's home family alone (docs/protocol.md, "Several
    /// families") — which is why the fan-out below can be told to skip it.
    multi: bool,
    tx: mpsc::Sender<ServerFrame>,
    /// `Some(close_code)` asks the connection task to close the socket.
    close_tx: Arc<watch::Sender<Option<u16>>>,
}

/// What a freshly registered connection task receives back.
pub struct Registration {
    /// Process-unique id; identifies the origin connection during fan-out.
    pub conn_id: u64,
    /// Frames queued for this connection by the registry.
    pub frames: mpsc::Receiver<ServerFrame>,
    /// Fires with a close code when the registry wants this socket gone.
    pub close: watch::Receiver<Option<u16>>,
}

pub struct Registry {
    next_conn_id: AtomicU64,
    queue_capacity: usize,
    shutdown: CancellationToken,
    inner: RwLock<HashMap<i64, Vec<ConnHandle>>>,
}

impl Registry {
    pub fn new(queue_capacity: usize) -> Self {
        Self {
            next_conn_id: AtomicU64::new(1),
            queue_capacity: queue_capacity.max(1),
            shutdown: CancellationToken::new(),
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// The token connection tasks watch for graceful shutdown; `main`
    /// cancels it on SIGTERM/SIGINT.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Add a connection for `user_id` and hand back its receiving ends.
    /// `multi` is whether the client said `families=1` on the upgrade.
    pub async fn register(&self, user_id: i64, session_id: i64, multi: bool) -> Registration {
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (tx, frames) = mpsc::channel(self.queue_capacity);
        let (close_tx, close) = watch::channel(None);
        let handle = ConnHandle {
            conn_id,
            session_id,
            multi,
            tx,
            close_tx: Arc::new(close_tx),
        };
        self.inner
            .write()
            .await
            .entry(user_id)
            .or_default()
            .push(handle);
        Registration {
            conn_id,
            frames,
            close,
        }
    }

    /// Remove a connection; called by its task on every exit path.
    pub async fn unregister(&self, user_id: i64, conn_id: u64) {
        let mut inner = self.inner.write().await;
        if let Some(conns) = inner.get_mut(&user_id) {
            conns.retain(|c| c.conn_id != conn_id);
            if conns.is_empty() {
                inner.remove(&user_id);
            }
        }
    }

    /// Send `frame` to every connection of every user in `user_ids`, except
    /// the connection `skip_conn` (the WS connection a `send` frame arrived
    /// on — it gets its `ack` directly instead).
    ///
    /// This used to hand back the users it found no connection for, and
    /// push targeting was derived from that list. It is not any more, and
    /// the list is gone rather than left lying about: "this user has a
    /// socket somewhere" is the wrong question — see `live_sessions`.
    pub async fn fan_out(&self, user_ids: &[i64], frame: &ServerFrame, skip_conn: Option<u64>) {
        self.fan_out_hiding(user_ids, frame, skip_conn, &HashSet::new())
            .await;
    }

    /// `fan_out`, except that the users in `hidden_from_legacy` are not sent
    /// the frame on any connection that did NOT say `families=1` — their
    /// multi-family connections still get it.
    ///
    /// The set is decided by the caller (`events::legacy_hidden`), because
    /// deciding it takes the database — each user's home family against the
    /// families the frame is about — and the registry never touches the
    /// database. An empty set is the plain fan-out.
    pub async fn fan_out_hiding(
        &self,
        user_ids: &[i64],
        frame: &ServerFrame,
        skip_conn: Option<u64>,
        hidden_from_legacy: &HashSet<i64>,
    ) {
        // Clone the handles under the read lock; deliver after releasing it.
        let targets: Vec<(i64, Vec<ConnHandle>)> = {
            let inner = self.inner.read().await;
            user_ids
                .iter()
                .map(|uid| (*uid, inner.get(uid).cloned().unwrap_or_default()))
                .collect()
        };
        for (user_id, conns) in targets {
            let hidden = hidden_from_legacy.contains(&user_id);
            for conn in conns {
                if Some(conn.conn_id) == skip_conn {
                    continue;
                }
                if hidden && !conn.multi {
                    continue;
                }
                match conn.tx.try_send(frame.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        // The consumer can't keep up; drop it rather than
                        // buffering unboundedly. REST resync recovers.
                        warn!(
                            user_id,
                            conn_id = conn.conn_id,
                            "websocket send queue full; kicking slow connection"
                        );
                        let _ = conn.close_tx.send(Some(CLOSE_GOING_AWAY));
                    }
                    // Connection task already exited; unregister is racing us.
                    Err(TrySendError::Closed(_)) => {}
                }
            }
        }
    }

    /// Send one frame to every connection of the listed users.
    pub async fn send_to_users(&self, user_ids: &[i64], frame: &ServerFrame) {
        self.fan_out(user_ids, frame, None).await;
    }

    /// Which of `user_ids` hold at least one connection that did NOT say
    /// `families=1` — the only users a family-scoped frame needs a database
    /// question asked about. Empty, as it will be once every app is updated,
    /// means no scoping work at all.
    pub async fn legacy_users(&self, user_ids: &[i64]) -> Vec<i64> {
        let inner = self.inner.read().await;
        user_ids
            .iter()
            .copied()
            .filter(|user_id| {
                inner
                    .get(user_id)
                    .is_some_and(|conns| conns.iter().any(|conn| !conn.multi))
            })
            .collect()
    }

    /// Is this ONE connection an app that predates several families?
    /// `None` when the connection has gone.
    pub async fn is_legacy_conn(&self, user_id: i64, conn_id: u64) -> Option<bool> {
        let inner = self.inner.read().await;
        inner
            .get(&user_id)
            .and_then(|conns| conns.iter().find(|c| c.conn_id == conn_id))
            .map(|conn| !conn.multi)
    }

    /// Send one frame to a SINGLE connection of `user_id` — the one that
    /// placed a call, which is the only one `call_ringing` is for. A no-op
    /// if that connection has since gone (the call will ring out).
    pub async fn send_to_conn(&self, user_id: i64, conn_id: u64, frame: &ServerFrame) {
        let handle = {
            let inner = self.inner.read().await;
            inner
                .get(&user_id)
                .and_then(|conns| conns.iter().find(|c| c.conn_id == conn_id).cloned())
        };
        if let Some(conn) = handle {
            let _ = conn.tx.try_send(frame.clone());
        }
    }

    /// Ask every connection authenticated by `session_id` to close (logout:
    /// the session is gone, so the code is 4401 — the client returns to
    /// login instead of reconnecting in a loop).
    pub async fn close_session(&self, session_id: i64) {
        let inner = self.inner.read().await;
        for conns in inner.values() {
            for conn in conns {
                if conn.session_id == session_id {
                    let _ = conn.close_tx.send(Some(CLOSE_SESSION_GONE));
                }
            }
        }
    }

    /// Which SESSIONS of the listed users have a socket right now.
    ///
    /// This is what push targeting asks, because a session is what one
    /// device is: a device registers itself with a bearer token, its socket
    /// authenticates with the same token, and both name the same session
    /// row. Asking per user instead — "is this person connected anywhere" —
    /// let one Mac left running answer for every phone on the account and
    /// silence all of them (docs/protocol.md, "Push notifications").
    ///
    /// The whole batch is sampled under ONE read lock, the way `fan_out`
    /// clones its targets, so a single push decision sees one instant
    /// rather than a user's socket appearing halfway down the list.
    pub async fn live_sessions(&self, user_ids: &[i64]) -> HashSet<i64> {
        let inner = self.inner.read().await;
        user_ids
            .iter()
            .filter_map(|user_id| inner.get(user_id))
            .flat_map(|conns| conns.iter().map(|conn| conn.session_id))
            .collect()
    }

    /// `live_sessions`, counting only connections that said `families=1`.
    ///
    /// For a push whose FRAME is held back from apps that predate several
    /// families (an invitation): such an app's socket was never sent the
    /// frame, so it must not be what decides its device needs no push.
    pub async fn live_multi_sessions(&self, user_ids: &[i64]) -> HashSet<i64> {
        let inner = self.inner.read().await;
        user_ids
            .iter()
            .filter_map(|user_id| inner.get(user_id))
            .flat_map(|conns| conns.iter().filter(|conn| conn.multi))
            .map(|conn| conn.session_id)
            .collect()
    }

    /// Number of live connections — used by main's bounded shutdown drain.
    pub async fn connection_count(&self) -> usize {
        self.inner.read().await.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pong() -> ServerFrame {
        ServerFrame::Pong
    }

    /// The sessions come back as a set, so the assertions sort into a Vec.
    fn sorted(sessions: HashSet<i64>) -> Vec<i64> {
        let mut sessions: Vec<i64> = sessions.into_iter().collect();
        sessions.sort_unstable();
        sessions
    }

    #[tokio::test]
    async fn live_multi_sessions_ignores_a_session_whose_only_socket_is_legacy() {
        let registry = Registry::new(4);
        let _old_phone = registry.register(1, 100, false).await;
        let _new_mac = registry.register(1, 101, true).await;
        assert_eq!(sorted(registry.live_multi_sessions(&[1]).await), vec![101]);
        assert_eq!(sorted(registry.live_sessions(&[1]).await), vec![100, 101]);
    }

    #[tokio::test]
    async fn live_sessions_holds_every_session_a_user_is_connected_on() {
        let registry = Registry::new(4);
        let mac = registry.register(1, 100, true).await;
        let _phone = registry.register(1, 101, true).await;
        assert_eq!(sorted(registry.live_sessions(&[1]).await), vec![100, 101]);
        registry.unregister(1, mac.conn_id).await;
        assert_eq!(
            sorted(registry.live_sessions(&[1]).await),
            vec![101],
            "closing one device's socket must not take the other's session with it"
        );
    }

    #[tokio::test]
    async fn live_sessions_covers_the_batch_and_skips_users_with_no_socket() {
        let registry = Registry::new(4);
        let _one = registry.register(1, 100, true).await;
        let _two = registry.register(2, 200, true).await;
        assert_eq!(
            sorted(registry.live_sessions(&[1, 2, 3]).await),
            vec![100, 200]
        );
        assert_eq!(
            sorted(registry.live_sessions(&[3]).await),
            Vec::<i64>::new()
        );
    }

    #[tokio::test]
    async fn fan_out_skips_the_origin_connection_but_reaches_other_devices() {
        let registry = Registry::new(4);
        let mut a = registry.register(1, 100, true).await;
        let mut b = registry.register(1, 101, true).await;
        registry.fan_out(&[1], &pong(), Some(a.conn_id)).await;
        assert!(
            b.frames.try_recv().is_ok(),
            "the user's other device must receive the frame"
        );
        assert!(
            a.frames.try_recv().is_err(),
            "the origin connection must not receive the fan-out frame"
        );
    }

    #[tokio::test]
    async fn a_full_send_queue_triggers_a_kick_instead_of_blocking() {
        let registry = Registry::new(1);
        let mut reg = registry.register(1, 100, true).await;
        registry.send_to_users(&[1], &pong()).await; // fills the queue
        registry.send_to_users(&[1], &pong()).await; // overflows -> kick
        assert_eq!(
            *reg.close.borrow_and_update(),
            Some(CLOSE_GOING_AWAY),
            "overflow must request a close with 1001"
        );
    }

    #[tokio::test]
    async fn close_session_signals_exactly_the_matching_connections() {
        let registry = Registry::new(4);
        let mut same = registry.register(1, 100, true).await;
        let mut other = registry.register(1, 200, true).await;
        registry.close_session(100).await;
        assert_eq!(*same.close.borrow_and_update(), Some(CLOSE_SESSION_GONE));
        assert_eq!(*other.close.borrow_and_update(), None);
    }

    /// A legacy connection of a hidden user is skipped; the same user's
    /// multi-family connection, and every connection of a user not hidden,
    /// still get the frame (docs/protocol.md, "Several families").
    #[tokio::test]
    async fn a_hidden_user_loses_the_frame_only_on_legacy_connections() {
        let registry = Registry::new(4);
        let mut old_app = registry.register(1, 100, false).await;
        let mut new_app = registry.register(1, 101, true).await;
        let mut other = registry.register(2, 200, false).await;
        assert_eq!(registry.legacy_users(&[1, 2, 3]).await, vec![1, 2]);
        let hidden: HashSet<i64> = [1].into_iter().collect();
        registry
            .fan_out_hiding(&[1, 2], &pong(), None, &hidden)
            .await;
        assert!(old_app.frames.try_recv().is_err(), "the old app is skipped");
        assert!(new_app.frames.try_recv().is_ok(), "the new app still hears");
        assert!(other.frames.try_recv().is_ok(), "a user not hidden hears");
        assert_eq!(
            registry.is_legacy_conn(1, old_app.conn_id).await,
            Some(true)
        );
        assert_eq!(
            registry.is_legacy_conn(1, new_app.conn_id).await,
            Some(false)
        );
    }

    #[tokio::test]
    async fn unregister_removes_the_connection_and_empties_the_map() {
        let registry = Registry::new(4);
        let reg = registry.register(1, 100, true).await;
        assert_eq!(registry.connection_count().await, 1);
        registry.unregister(1, reg.conn_id).await;
        assert_eq!(registry.connection_count().await, 0);
        assert_eq!(
            sorted(registry.live_sessions(&[1]).await),
            Vec::<i64>::new()
        );
    }
}
