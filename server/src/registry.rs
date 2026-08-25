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
    pub async fn register(&self, user_id: i64, session_id: i64) -> Registration {
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (tx, frames) = mpsc::channel(self.queue_capacity);
        let (close_tx, close) = watch::channel(None);
        let handle = ConnHandle {
            conn_id,
            session_id,
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
        // Clone the handles under the read lock; deliver after releasing it.
        let targets: Vec<(i64, Vec<ConnHandle>)> = {
            let inner = self.inner.read().await;
            user_ids
                .iter()
                .map(|uid| (*uid, inner.get(uid).cloned().unwrap_or_default()))
                .collect()
        };
        for (user_id, conns) in targets {
            for conn in conns {
                if Some(conn.conn_id) == skip_conn {
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
    async fn live_sessions_holds_every_session_a_user_is_connected_on() {
        let registry = Registry::new(4);
        let mac = registry.register(1, 100).await;
        let _phone = registry.register(1, 101).await;
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
        let _one = registry.register(1, 100).await;
        let _two = registry.register(2, 200).await;
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
        let mut a = registry.register(1, 100).await;
        let mut b = registry.register(1, 101).await;
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
        let mut reg = registry.register(1, 100).await;
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
        let mut same = registry.register(1, 100).await;
        let mut other = registry.register(1, 200).await;
        registry.close_session(100).await;
        assert_eq!(*same.close.borrow_and_update(), Some(CLOSE_SESSION_GONE));
        assert_eq!(*other.close.borrow_and_update(), None);
    }

    #[tokio::test]
    async fn unregister_removes_the_connection_and_empties_the_map() {
        let registry = Registry::new(4);
        let reg = registry.register(1, 100).await;
        assert_eq!(registry.connection_count().await, 1);
        registry.unregister(1, reg.conn_id).await;
        assert_eq!(registry.connection_count().await, 0);
        assert_eq!(
            sorted(registry.live_sessions(&[1]).await),
            Vec::<i64>::new()
        );
    }
}
