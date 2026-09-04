//! The calls in flight: a small in-memory state machine for voice-call
//! signalling (docs/protocol.md, "Voice calls").
//!
//! Nothing here touches the database or a socket. The registry answers
//! questions — "may this call begin", "who does this candidate go to",
//! "what did this call end as" — and hands back what the caller must
//! DELIVER and RECORD; `handlers_call` does the delivering and `ws` does the
//! routing. That split is what makes every transition unit-testable with
//! nothing but a paused tokio clock.
//!
//! A call does not survive a restart, on purpose. Its audio is peer to peer
//! and continues without the server; what a restart loses is the record of
//! how it ended, and the busy state that would otherwise have to be
//! reconciled against sockets that no longer exist.
//!
//! One `std::sync::Mutex`, never held across an `.await`: every method
//! takes the lock, mutates, and returns plain values.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use tokio::time::{Duration, Instant};
use uuid::Uuid;

use crate::models::IceCandidate;

/// How many of the caller's candidates are kept while a call rings, for
/// replay to a callee device that connects late (protocol.md fixes 64).
pub const MAX_BUFFERED_CANDIDATES: usize = 64;

/// An answered call with NEITHER party connected for this long is ended as
/// `failed`. A backstop, not the rule: the audio is peer to peer and
/// outlives any number of socket reconnects, so one side dropping its
/// socket must never hang a healthy call up (protocol.md fixes 60 s).
pub const DETACHED_GRACE: Duration = Duration::from_secs(60);

/// How long a finished call is remembered for the `call_end` replay to a
/// callee device that connects after it ended (protocol.md: two minutes).
pub const RECENT_END_WINDOW: Duration = Duration::from_secs(120);

/// Bound on the recent-end ring buffer; far above anything a family does.
const RECENT_END_CAP: usize = 256;

/// Why a call ended, as the `reason` on a `call_end` frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Hangup,
    Decline,
    Cancel,
    Timeout,
    Failed,
    AnsweredElsewhere,
}

impl EndReason {
    /// The wire word.
    pub fn as_str(self) -> &'static str {
        match self {
            EndReason::Hangup => "hangup",
            EndReason::Decline => "decline",
            EndReason::Cancel => "cancel",
            EndReason::Timeout => "timeout",
            EndReason::Failed => "failed",
            EndReason::AnsweredElsewhere => "answered_elsewhere",
        }
    }

    /// The four reasons a CLIENT may send. `timeout` and
    /// `answered_elsewhere` are the server's to say.
    pub fn from_client(reason: &str) -> Option<Self> {
        match reason {
            "hangup" => Some(EndReason::Hangup),
            "decline" => Some(EndReason::Decline),
            "cancel" => Some(EndReason::Cancel),
            "failed" => Some(EndReason::Failed),
            _ => None,
        }
    }
}

/// What the record says a call was (protocol.md, the `Call` object).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Missed,
    Declined,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Completed => "completed",
            Outcome::Missed => "missed",
            Outcome::Declined => "declined",
            Outcome::Failed => "failed",
        }
    }

    /// The English placeholder body of the record — what a client that
    /// predates calls shows, and what a client that knows the `call`
    /// object never does (protocol.md, "The record"). A video call's says
    /// so, the same old-client courtesy the voice wording is
    /// (protocol.md, "Video").
    pub fn placeholder_body(self, video: bool) -> &'static str {
        match (self, video) {
            (Outcome::Missed, false) => "Missed voice call",
            (Outcome::Missed, true) => "Missed video call",
            (_, false) => "Voice call",
            (_, true) => "Video call",
        }
    }
}

/// Everything the caller needs to deliver a `call_end` and write the
/// record for a call that is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub call_id: Uuid,
    pub chat_id: i64,
    pub caller_id: i64,
    pub callee_id: i64,
    pub reason: EndReason,
    pub outcome: Outcome,
    /// Seconds from the answer to the end; present iff the call was ever
    /// answered.
    pub duration_secs: Option<i64>,
    /// Whether it was a VIDEO call — fixed at `begin`, carried through so
    /// the record says what kind of call it was (protocol.md, "Video").
    pub video: bool,
    /// The callee never heard this call ring, because they had blocked the
    /// caller. Carried out of the registry so `finish_call` knows not to
    /// send them a `call_end` for a call they were never told about —
    /// which would be a frame arriving out of nowhere, and the loudest
    /// possible tell (protocol.md, "Blocking a member").
    ///
    /// The RECORD is written regardless: the call was accepted, it rang
    /// out, and the caller gets the ordinary `missed` entry that makes the
    /// whole thing indistinguishable from being ignored.
    pub suppressed: bool,
}

/// Why an offer was refused: one call per person, on either side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Caller,
    Callee,
}

/// Why an answer was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFault {
    NotFound,
    /// The answerer is not the callee of this call.
    NotCallee,
    /// The call is no longer ringing (already answered).
    NotRinging,
}

/// The parties of a call that was just answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub chat_id: i64,
    pub caller_id: i64,
    pub callee_id: i64,
}

/// Where a candidate goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub to_user: i64,
}

/// A ringing call as replayed to a callee connection that arrives late:
/// the offer, then the candidates the caller gathered meanwhile, in the
/// order they were gathered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOffer {
    pub call_id: Uuid,
    pub chat_id: i64,
    pub from_user_id: i64,
    pub sdp: String,
    /// A late callee device must ring as what the call IS — a video call's
    /// replayed offer carries the flag exactly as the live one did
    /// (protocol.md, "Video").
    pub video: bool,
    pub candidates: Vec<IceCandidate>,
}

#[derive(Debug)]
enum Phase {
    Ringing {
        since: Instant,
        offer_sdp: String,
        caller_candidates: Vec<IceCandidate>,
    },
    Active {
        answered_at: Instant,
        /// Set while NEITHER party has a socket; cleared the moment one
        /// comes back.
        detached_since: Option<Instant>,
    },
}

#[derive(Debug)]
struct Call {
    id: Uuid,
    chat_id: i64,
    caller_id: i64,
    callee_id: i64,
    /// The connection the offer came in on. A ringing call whose origin
    /// closes is cancelled — nothing should ring for somebody who is no
    /// longer there. Irrelevant once answered: an answered call's frames are
    /// addressed to USERS, so a reconnect just works.
    caller_conn: u64,
    /// A VIDEO call, decided at `begin` and never renegotiated: cameras
    /// toggle mid-call, the call's kind does not (protocol.md, "Video").
    video: bool,
    /// The callee has BLOCKED the caller, so this call rings only
    /// nominally: it is parked here so it can ring out and leave the
    /// caller the ordinary `missed` record, but no frame and no push ever
    /// reach the callee (protocol.md, "Blocking a member").
    ///
    /// The callee is therefore NOT a party to it — see [`Call::involves`],
    /// which every busy scan goes through. Without that, a blocked person
    /// could keep somebody uncallable by their whole family with a loop of
    /// offers, and a third member calling them would be told `peer_busy`
    /// for a call nobody is in.
    suppressed: bool,
    phase: Phase,
}

impl Call {
    /// Is `user` really a party to this call?
    ///
    /// The caller always is. The callee is too, UNLESS the call is
    /// suppressed — in which case they have not been told it exists and
    /// must not be treated as busy by it.
    fn involves(&self, user: i64) -> bool {
        self.caller_id == user || (self.callee_id == user && !self.suppressed)
    }
}

#[derive(Debug)]
struct RecentEnd {
    call_id: Uuid,
    callee_id: i64,
    reason: EndReason,
    at: Instant,
}

#[derive(Debug, Default)]
struct Inner {
    calls: HashMap<Uuid, Call>,
    recent_ends: VecDeque<RecentEnd>,
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct CallRegistry {
    inner: Mutex<Inner>,
}

impl CallRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned lock means a panic mid-mutation; the state is simple
        // enough that carrying on is safer than taking every call down.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether `user` is a party to any call in flight.
    pub fn is_busy(&self, user: i64) -> bool {
        self.lock().calls.values().any(|call| call.involves(user))
    }

    /// Start ringing. Refused when either party is on a call already —
    /// and a re-used `call_id` counts as the caller being on that call.
    #[allow(clippy::too_many_arguments)] // the flag is part of the offer, not a config
    pub fn begin(
        &self,
        call_id: Uuid,
        chat_id: i64,
        caller_id: i64,
        callee_id: i64,
        caller_conn: u64,
        offer_sdp: String,
        video: bool,
        suppressed: bool,
    ) -> Result<(), Busy> {
        let mut inner = self.lock();
        for call in inner.calls.values() {
            if call.id == call_id || call.involves(caller_id) {
                return Err(Busy::Caller);
            }
        }
        if inner.calls.values().any(|call| call.involves(callee_id)) {
            return Err(Busy::Callee);
        }
        inner.calls.insert(
            call_id,
            Call {
                id: call_id,
                chat_id,
                caller_id,
                callee_id,
                caller_conn,
                video,
                suppressed,
                phase: Phase::Ringing {
                    since: Instant::now(),
                    offer_sdp,
                    caller_candidates: Vec::new(),
                },
            },
        );
        Ok(())
    }

    /// The callee takes the call. Only the callee, only while it rings.
    pub fn answer(&self, call_id: Uuid, by_user: i64) -> Result<Answered, CallFault> {
        let mut inner = self.lock();
        let call = inner.calls.get_mut(&call_id).ok_or(CallFault::NotFound)?;
        if call.callee_id != by_user {
            return Err(CallFault::NotCallee);
        }
        if !matches!(call.phase, Phase::Ringing { .. }) {
            return Err(CallFault::NotRinging);
        }
        call.phase = Phase::Active {
            answered_at: Instant::now(),
            detached_since: None,
        };
        Ok(Answered {
            chat_id: call.chat_id,
            caller_id: call.caller_id,
            callee_id: call.callee_id,
        })
    }

    /// Accept a candidate from a party and say who it is relayed to. The
    /// caller's candidates are buffered while the call rings, for the
    /// replay to a late callee device; the callee's are never buffered —
    /// the caller has been connected since the offer. A candidate for an
    /// unknown call, or from somebody who is not a party, goes nowhere.
    ///
    /// A SUPPRESSED call relays nothing towards the callee. The suppression
    /// is total and covers "the live and the buffered `call_ice`" as well
    /// as the offer (protocol.md, "Blocking a member"): a blocked caller
    /// goes on gathering candidates for the full ring timeout, exactly as
    /// they would against a phone nobody picks up, and every one of them
    /// stops here. The candidates are still BUFFERED above, because
    /// buffering is what a late device replay reads and that replay is
    /// itself suppressed — dropping them at the buffer instead would make
    /// this depend on two places agreeing.
    pub fn add_candidate(
        &self,
        call_id: Uuid,
        from_user: i64,
        candidate: IceCandidate,
    ) -> Option<Relay> {
        let mut inner = self.lock();
        let call = inner.calls.get_mut(&call_id)?;
        if from_user == call.caller_id {
            let suppressed = call.suppressed;
            if let Phase::Ringing {
                caller_candidates, ..
            } = &mut call.phase
            {
                if caller_candidates.len() >= MAX_BUFFERED_CANDIDATES {
                    caller_candidates.remove(0);
                }
                caller_candidates.push(candidate);
            }
            if suppressed {
                return None;
            }
            Some(Relay {
                to_user: call.callee_id,
            })
        } else if from_user == call.callee_id {
            Some(Relay {
                to_user: call.caller_id,
            })
        } else {
            None
        }
    }

    /// End a call, on behalf of `by_user` (a party — anyone else is
    /// ignored) or on the server's own authority (`None`). Answers with
    /// what to deliver and record, or `None` for a call the registry does
    /// not hold.
    ///
    /// The outcome follows the phase as much as the reason: a `hangup`
    /// before the answer is the caller giving up (`missed`) or the callee
    /// refusing (`declined`), whatever the client called it.
    pub fn end(&self, call_id: Uuid, by_user: Option<i64>, reason: EndReason) -> Option<Ended> {
        let mut inner = self.lock();
        let is_party = match (&by_user, inner.calls.get(&call_id)) {
            (_, None) => return None,
            (None, Some(_)) => true,
            (Some(user), Some(call)) => call.caller_id == *user || call.callee_id == *user,
        };
        if !is_party {
            return None;
        }
        let call = inner.calls.remove(&call_id)?;
        let ended = Self::ended(&call, by_user, reason);
        inner.recent_ends.push_back(RecentEnd {
            call_id,
            // A suppressed call records NO callee, so its `call_end` is
            // never replayed to somebody who was never told it rang. `-1`
            // matches no user; the row is still pushed so the window's
            // eviction stays uniform.
            callee_id: if call.suppressed { -1 } else { call.callee_id },
            reason: ended.reason,
            at: Instant::now(),
        });
        while inner.recent_ends.len() > RECENT_END_CAP {
            inner.recent_ends.pop_front();
        }
        Some(ended)
    }

    fn ended(call: &Call, by_user: Option<i64>, reason: EndReason) -> Ended {
        let (outcome, duration_secs) = match &call.phase {
            Phase::Ringing { .. } => {
                let outcome = match reason {
                    EndReason::Decline => Outcome::Declined,
                    EndReason::Failed => Outcome::Failed,
                    // A hangup while ringing is whichever side sent it
                    // giving up; the record reads accordingly.
                    EndReason::Hangup if by_user == Some(call.callee_id) => Outcome::Declined,
                    EndReason::Hangup
                    | EndReason::Cancel
                    | EndReason::Timeout
                    | EndReason::AnsweredElsewhere => Outcome::Missed,
                };
                (outcome, None)
            }
            Phase::Active { answered_at, .. } => {
                let duration = answered_at.elapsed().as_secs() as i64;
                let outcome = match reason {
                    EndReason::Failed => Outcome::Failed,
                    _ => Outcome::Completed,
                };
                (outcome, Some(duration))
            }
        };
        Ended {
            call_id: call.id,
            chat_id: call.chat_id,
            caller_id: call.caller_id,
            callee_id: call.callee_id,
            reason,
            outcome,
            duration_secs,
            video: call.video,
            suppressed: call.suppressed,
        }
    }

    /// The ringing call `user` is being rung on, for the replay to a
    /// connection that registers while it rings.
    pub fn pending_offer_for(&self, user: i64) -> Option<PendingOffer> {
        let inner = self.lock();
        inner.calls.values().find_map(|call| match &call.phase {
            Phase::Ringing {
                offer_sdp,
                caller_candidates,
                ..
            }
            // `involves` rather than a bare `callee_id ==`: a suppressed
            // call must not be replayed to the callee either. This is the
            // one path where a call frame reaches a user without passing
            // through `handlers_call`, and without this test a device that
            // connects mid-ring would be handed the offer the live path
            // deliberately withheld — the block would hold until somebody
            // opened their laptop.
            if call.involves(user) && call.callee_id == user => Some(PendingOffer {
                call_id: call.id,
                chat_id: call.chat_id,
                from_user_id: call.caller_id,
                sdp: offer_sdp.clone(),
                video: call.video,
                candidates: caller_candidates.clone(),
            }),
            _ => None,
        })
    }

    /// Calls `user` was the callee of that ended within the replay window,
    /// oldest first — so a device woken for a call that was meanwhile
    /// taken elsewhere stops ringing at once.
    pub fn recent_ends_for(&self, user: i64) -> Vec<(Uuid, EndReason)> {
        let mut inner = self.lock();
        let cutoff = Instant::now() - RECENT_END_WINDOW;
        while inner.recent_ends.front().is_some_and(|end| end.at < cutoff) {
            inner.recent_ends.pop_front();
        }
        inner
            .recent_ends
            .iter()
            .filter(|end| end.callee_id == user)
            .map(|end| (end.call_id, end.reason))
            .collect()
    }

    /// The connection that placed a ringing call has closed: the call is
    /// cancelled. Answered calls are untouched — they are addressed to users
    /// and survive a reconnect.
    pub fn connection_closed(&self, conn_id: u64) -> Vec<Ended> {
        let ids: Vec<Uuid> = self
            .lock()
            .calls
            .values()
            .filter(|call| {
                call.caller_conn == conn_id && matches!(call.phase, Phase::Ringing { .. })
            })
            .map(|call| call.id)
            .collect();
        ids.into_iter()
            .filter_map(|id| self.end(id, None, EndReason::Cancel))
            .collect()
    }

    /// Every user who is a party to an answered call — what the sweeper
    /// asks the connection registry about.
    pub fn active_parties(&self) -> Vec<i64> {
        let inner = self.lock();
        let mut users: HashSet<i64> = HashSet::new();
        for call in inner.calls.values() {
            if matches!(call.phase, Phase::Active { .. }) {
                users.insert(call.caller_id);
                if !call.suppressed {
                    users.insert(call.callee_id);
                }
            }
        }
        users.into_iter().collect()
    }

    /// The periodic pass: ring-outs, and the detached backstop for
    /// answered calls. `connected` holds every user (of `active_parties`)
    /// who has a socket right now.
    pub fn sweep(&self, ring_timeout: Duration, connected: &HashSet<i64>) -> Vec<Ended> {
        let now = Instant::now();
        let mut to_end: Vec<(Uuid, EndReason)> = Vec::new();
        {
            let mut inner = self.lock();
            for call in inner.calls.values_mut() {
                match &mut call.phase {
                    Phase::Ringing { since, .. } => {
                        if now.duration_since(*since) >= ring_timeout {
                            to_end.push((call.id, EndReason::Timeout));
                        }
                    }
                    Phase::Active { detached_since, .. } => {
                        let anyone = connected.contains(&call.caller_id)
                            || connected.contains(&call.callee_id);
                        if anyone {
                            *detached_since = None;
                        } else {
                            let since = *detached_since.get_or_insert(now);
                            if now.duration_since(since) >= DETACHED_GRACE {
                                to_end.push((call.id, EndReason::Failed));
                            }
                        }
                    }
                }
            }
        }
        to_end
            .into_iter()
            .filter_map(|(id, reason)| self.end(id, None, reason))
            .collect()
    }
}

/// The periodic pass that ends calls the clients cannot: ring-outs and the
/// detached-answered backstop (docs/protocol.md, "Voice calls"). One tick a
/// second — the timeouts are coarse (45 s, 60 s) so a finer cadence would
/// only cost wake-ups. Cancelled on shutdown via the registry's token.
///
/// Lives here rather than in `handlers_call` because it is the registry's
/// clock; the finishing (deliver + record) is delegated back to
/// `handlers_call::finish_call`, which is the seam to the database.
pub fn spawn_sweeper(state: crate::state::AppState) {
    use tokio::time::Duration;
    let shutdown = state.registry.shutdown_token();
    let ring_timeout = Duration::from_secs(state.cfg.calls.ring_timeout_secs);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    // Which parties of answered calls have a socket right
                    // now — only the detached backstop needs it, and there
                    // are rarely any at all.
                    let parties = state.calls.active_parties();
                    let connected = connected_users(&state, &parties).await;
                    for ended in state.calls.sweep(ring_timeout, &connected) {
                        crate::handlers_call::finish_call(&state, ended).await;
                    }
                }
                _ = shutdown.cancelled() => break,
            }
        }
    });
}

/// Which of `parties` has at least one live socket. `live_sessions` answers
/// in sessions; a user is connected if any of theirs is present, so this
/// asks per user (a handful at a time — only parties to answered calls).
async fn connected_users(
    state: &crate::state::AppState,
    parties: &[i64],
) -> std::collections::HashSet<i64> {
    let mut connected = std::collections::HashSet::new();
    for &user in parties {
        if !state.registry.live_sessions(&[user]).await.is_empty() {
            connected.insert(user);
        }
    }
    connected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn candidate(n: u16) -> IceCandidate {
        IceCandidate {
            candidate: format!("candidate:{n}"),
            sdp_mid: Some("0".to_string()),
            sdp_mline_index: Some(0),
        }
    }

    fn ring(reg: &CallRegistry, call: u128, caller: i64, callee: i64, conn: u64) {
        reg.begin(
            id(call),
            42,
            caller,
            callee,
            conn,
            "offer".to_string(),
            false,
            false,
        )
        .expect("begins");
    }

    #[test]
    fn one_call_per_person_on_either_side() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        assert!(reg.is_busy(7) && reg.is_busy(9) && !reg.is_busy(11));
        // The caller, from another device, is busy with their own call.
        assert_eq!(
            reg.begin(id(2), 43, 7, 11, 101, "o".into(), false, false),
            Err(Busy::Caller)
        );
        // Somebody ringing the callee is refused as peer busy.
        assert_eq!(
            reg.begin(id(3), 44, 11, 9, 102, "o".into(), false, false),
            Err(Busy::Callee)
        );
        // And somebody ringing the CALLER (who is a caller, not a callee)
        // is refused too: busy is about being on a call at all.
        assert_eq!(
            reg.begin(id(4), 45, 11, 7, 102, "o".into(), false, false),
            Err(Busy::Callee)
        );
        // A re-used id is the caller being on that call.
        assert_eq!(
            reg.begin(id(1), 46, 13, 15, 103, "o".into(), false, false),
            Err(Busy::Caller)
        );
        // Unrelated people call freely.
        reg.begin(id(5), 47, 13, 15, 104, "o".into(), false, false)
            .expect("an unrelated pair may talk");
    }

    /// A suppressed call — one the callee has blocked into — parks a real
    /// call so it can ring out, but the CALLEE is not a party to it.
    ///
    /// Without this the block hands the blocked person a weapon: a loop of
    /// offers keeps somebody uncallable by their whole family, and a third
    /// member ringing them is told `peer_busy` for a call nobody is in.
    #[test]
    fn a_suppressed_call_does_not_make_the_callee_busy() {
        let reg = CallRegistry::new();
        reg.begin(id(1), 42, 7, 9, 100, "o".into(), false, true)
            .expect("a suppressed call still begins");

        // The CALLER is genuinely busy — they placed it, they see it
        // ringing, and letting them place a second would tell them this
        // one is not real.
        assert!(reg.is_busy(7), "the caller is on a call");
        assert_eq!(
            reg.begin(id(2), 43, 7, 11, 101, "o".into(), false, false),
            Err(Busy::Caller)
        );

        // The CALLEE is not busy at all.
        assert!(!reg.is_busy(9), "the callee was never told this rang");
        // They may place a call of their own...
        reg.begin(id(3), 44, 9, 11, 102, "o".into(), false, false)
            .expect("the callee may still call out");
        // ...and a third member may ring them: no `peer_busy` on account
        // of a call they cannot hear.
        let reg2 = CallRegistry::new();
        reg2.begin(id(1), 42, 7, 9, 100, "o".into(), false, true)
            .expect("suppressed");
        reg2.begin(id(4), 45, 11, 9, 103, "o".into(), false, false)
            .expect("a third member reaches them regardless");
    }

    /// A blocked caller goes on gathering ICE candidates for the whole
    /// ring timeout, exactly as they would against a phone nobody picks
    /// up. Every one of them must stop at the server.
    ///
    /// The suppression is TOTAL and covers "the live and the buffered
    /// `call_ice`" as well as the offer (protocol.md, "Blocking a
    /// member"). Relaying them would put frames on the blocker's socket
    /// for a call they were never told about — and a client that logged or
    /// counted them would have the block in its hands.
    #[test]
    fn a_suppressed_calls_ice_candidates_are_never_relayed_to_the_callee() {
        let reg = CallRegistry::new();
        reg.begin(id(1), 42, 7, 9, 100, "o".into(), false, true)
            .expect("suppressed");
        assert!(
            reg.add_candidate(id(1), 7, candidate(1)).is_none(),
            "a suppressed caller's candidate relays nowhere",
        );

        // The control: the SAME call unsuppressed relays to the callee, so
        // the test above is pinned to the flag and not to some other
        // reason the relay declined.
        let open = CallRegistry::new();
        open.begin(id(2), 42, 7, 9, 100, "o".into(), false, false)
            .expect("an ordinary call");
        assert_eq!(
            open.add_candidate(id(2), 7, candidate(1))
                .map(|r| r.to_user),
            Some(9),
        );
    }

    /// The blocked caller hanging up while it rings is the path a client
    /// takes, not the sweeper — and it used to fan `call_end` to both
    /// parties unconditionally.
    ///
    /// A `call_end` for a call the callee never heard ring is a frame
    /// arriving out of nowhere: the loudest possible tell. The caller
    /// still gets it, and still gets the ordinary `missed` record, because
    /// their experience must stay identical to ringing a phone nobody
    /// picks up.
    #[test]
    fn a_suppressed_call_ends_for_the_caller_alone() {
        let reg = CallRegistry::new();
        reg.begin(id(1), 42, 7, 9, 100, "o".into(), false, true)
            .expect("suppressed");
        let ended = reg
            .end(id(1), Some(7), EndReason::Hangup)
            .expect("the caller may give up");
        assert!(
            ended.suppressed,
            "the flag has to survive out of the registry, or the handler cannot filter on it",
        );

        // The control, again: an ordinary call ends for both.
        let open = CallRegistry::new();
        open.begin(id(2), 42, 7, 9, 100, "o".into(), false, false)
            .expect("an ordinary call");
        let ended = open.end(id(2), Some(7), EndReason::Hangup).expect("ends");
        assert!(!ended.suppressed);
    }

    /// The replays are the one path where a call frame reaches a user
    /// without passing through `handlers_call`, so the suppression has to
    /// hold here too — otherwise the block lasts until the callee opens a
    /// laptop and the offer is handed over late.
    #[test]
    fn a_suppressed_call_is_never_replayed_to_the_callee() {
        let reg = CallRegistry::new();
        reg.begin(id(1), 42, 7, 9, 100, "o".into(), false, true)
            .expect("suppressed");
        assert!(
            reg.pending_offer_for(9).is_none(),
            "a device connecting mid-ring must not be handed the withheld offer"
        );
        // The caller's own view is unaffected.
        let ended = reg.end(id(1), None, EndReason::Timeout).expect("ends");
        assert!(ended.suppressed, "the flag rides out with the call");
        assert!(
            reg.recent_ends_for(9).is_empty(),
            "no call_end is replayed for a call the callee never heard"
        );
    }

    #[test]
    fn only_the_callee_may_answer_and_only_once() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        assert_eq!(reg.answer(id(2), 9), Err(CallFault::NotFound));
        assert_eq!(reg.answer(id(1), 7), Err(CallFault::NotCallee));
        assert_eq!(reg.answer(id(1), 11), Err(CallFault::NotCallee));
        assert_eq!(
            reg.answer(id(1), 9),
            Ok(Answered {
                chat_id: 42,
                caller_id: 7,
                callee_id: 9
            })
        );
        // The callee's second device is too late.
        assert_eq!(reg.answer(id(1), 9), Err(CallFault::NotRinging));
    }

    #[test]
    fn candidates_are_relayed_to_the_other_party_and_the_callers_are_buffered_while_ringing() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        assert_eq!(
            reg.add_candidate(id(1), 7, candidate(1)),
            Some(Relay { to_user: 9 })
        );
        assert_eq!(
            reg.add_candidate(id(1), 9, candidate(2)),
            Some(Relay { to_user: 7 })
        );
        assert_eq!(reg.add_candidate(id(1), 11, candidate(3)), None);
        assert_eq!(reg.add_candidate(id(2), 7, candidate(4)), None);
        // Only the caller's candidate was kept, in order.
        let pending = reg.pending_offer_for(9).expect("ringing");
        assert_eq!(pending.candidates, vec![candidate(1)]);
        assert_eq!(pending.sdp, "offer");
        assert_eq!(pending.from_user_id, 7);
        assert_eq!(
            reg.pending_offer_for(7),
            None,
            "the caller is not being rung"
        );
        // After the answer nothing is buffered, but the relay goes on.
        reg.answer(id(1), 9).expect("answers");
        assert_eq!(
            reg.add_candidate(id(1), 7, candidate(5)),
            Some(Relay { to_user: 9 })
        );
        assert_eq!(reg.pending_offer_for(9), None);
    }

    #[test]
    fn the_candidate_buffer_keeps_the_most_recent_sixty_four() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        for n in 0..(MAX_BUFFERED_CANDIDATES as u16 + 10) {
            reg.add_candidate(id(1), 7, candidate(n));
        }
        let pending = reg.pending_offer_for(9).expect("ringing");
        assert_eq!(pending.candidates.len(), MAX_BUFFERED_CANDIDATES);
        assert_eq!(pending.candidates[0], candidate(10));
        assert_eq!(
            pending.candidates[MAX_BUFFERED_CANDIDATES - 1],
            candidate(MAX_BUFFERED_CANDIDATES as u16 + 9)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_outcome_follows_the_phase_as_much_as_the_reason() {
        // Ringing: a hangup from the caller is a cancel, from the callee a
        // decline; a stranger's end is ignored.
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        assert_eq!(reg.end(id(1), Some(11), EndReason::Hangup), None);
        let ended = reg.end(id(1), Some(7), EndReason::Hangup).expect("ended");
        assert_eq!(
            (ended.outcome, ended.duration_secs),
            (Outcome::Missed, None)
        );
        assert_eq!(ended.reason, EndReason::Hangup);
        assert_eq!(reg.end(id(1), Some(7), EndReason::Hangup), None, "gone");

        ring(&reg, 2, 7, 9, 100);
        let ended = reg.end(id(2), Some(9), EndReason::Hangup).expect("ended");
        assert_eq!(ended.outcome, Outcome::Declined);
        ring(&reg, 3, 7, 9, 100);
        assert_eq!(
            reg.end(id(3), Some(9), EndReason::Decline)
                .map(|e| e.outcome),
            Some(Outcome::Declined)
        );
        ring(&reg, 4, 7, 9, 100);
        assert_eq!(
            reg.end(id(4), Some(7), EndReason::Cancel)
                .map(|e| e.outcome),
            Some(Outcome::Missed)
        );
        ring(&reg, 5, 7, 9, 100);
        assert_eq!(
            reg.end(id(5), None, EndReason::Timeout).map(|e| e.outcome),
            Some(Outcome::Missed)
        );
        ring(&reg, 6, 7, 9, 100);
        assert_eq!(
            reg.end(id(6), Some(7), EndReason::Failed)
                .map(|e| e.outcome),
            Some(Outcome::Failed)
        );

        // Answered: the duration is measured from the answer, and only a
        // failure is not a completed call.
        ring(&reg, 7, 7, 9, 100);
        reg.answer(id(7), 9).expect("answers");
        tokio::time::advance(Duration::from_secs(222)).await;
        let ended = reg.end(id(7), Some(9), EndReason::Hangup).expect("ended");
        assert_eq!(
            (ended.outcome, ended.duration_secs),
            (Outcome::Completed, Some(222))
        );
        ring(&reg, 8, 7, 9, 100);
        reg.answer(id(8), 9).expect("answers");
        tokio::time::advance(Duration::from_secs(3)).await;
        let ended = reg.end(id(8), Some(7), EndReason::Failed).expect("ended");
        assert_eq!(
            (ended.outcome, ended.duration_secs),
            (Outcome::Failed, Some(3))
        );
    }

    #[test]
    fn a_closing_origin_connection_cancels_only_its_own_ringing_call() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        ring(&reg, 2, 11, 13, 100); // same connection id, different people
        ring(&reg, 3, 15, 17, 101);
        reg.answer(id(2), 13).expect("answers");
        let ended = reg.connection_closed(100);
        assert_eq!(ended.len(), 1, "the answered call survives the drop");
        assert_eq!(ended[0].call_id, id(1));
        assert_eq!(ended[0].reason, EndReason::Cancel);
        assert_eq!(ended[0].outcome, Outcome::Missed);
        assert!(reg.is_busy(11) && reg.is_busy(15));
        assert!(!reg.is_busy(7));
    }

    #[tokio::test(start_paused = true)]
    async fn the_sweep_rings_a_call_out_after_the_timeout() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        let nobody = HashSet::new();
        assert!(reg.sweep(Duration::from_secs(45), &nobody).is_empty());
        tokio::time::advance(Duration::from_secs(44)).await;
        assert!(reg.sweep(Duration::from_secs(45), &nobody).is_empty());
        tokio::time::advance(Duration::from_secs(1)).await;
        let ended = reg.sweep(Duration::from_secs(45), &nobody);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].reason, EndReason::Timeout);
        assert_eq!(ended[0].outcome, Outcome::Missed);
        assert!(!reg.is_busy(7));
    }

    #[tokio::test(start_paused = true)]
    async fn the_detached_backstop_fails_an_answered_call_nobody_is_connected_to() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        reg.answer(id(1), 9).expect("answers");
        assert_eq!(reg.active_parties().len(), 2);
        let nobody = HashSet::new();
        let caller_only: HashSet<i64> = [7].into_iter().collect();

        // Both gone: this sweep starts the detached clock.
        assert!(reg.sweep(Duration::from_secs(45), &nobody).is_empty());
        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(reg.sweep(Duration::from_secs(45), &nobody).is_empty());
        // One comes back: the clock resets, even though the other is gone.
        assert!(reg.sweep(Duration::from_secs(45), &caller_only).is_empty());
        // Both gone again: the NEXT nobody-sweep restarts the clock from
        // scratch, so the 60 s runs from here, not from the first outage.
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(reg.sweep(Duration::from_secs(45), &nobody).is_empty());
        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(reg.sweep(Duration::from_secs(45), &nobody).is_empty());
        tokio::time::advance(Duration::from_secs(1)).await;
        let ended = reg.sweep(Duration::from_secs(45), &nobody);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].reason, EndReason::Failed);
        assert_eq!(ended[0].outcome, Outcome::Failed);
        assert!(ended[0].duration_secs.is_some(), "it was answered");
        assert!(reg.active_parties().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn recent_ends_are_remembered_for_the_callee_for_two_minutes() {
        let reg = CallRegistry::new();
        ring(&reg, 1, 7, 9, 100);
        reg.answer(id(1), 9).expect("answers");
        reg.end(id(1), Some(7), EndReason::Hangup);
        ring(&reg, 2, 7, 11, 100);
        reg.end(id(2), None, EndReason::Timeout);
        assert_eq!(reg.recent_ends_for(9), vec![(id(1), EndReason::Hangup)]);
        assert_eq!(reg.recent_ends_for(11), vec![(id(2), EndReason::Timeout)]);
        assert!(
            reg.recent_ends_for(7).is_empty(),
            "the caller is not replayed to"
        );
        tokio::time::advance(RECENT_END_WINDOW + Duration::from_secs(1)).await;
        assert!(reg.recent_ends_for(9).is_empty());
        assert!(reg.recent_ends_for(11).is_empty());
    }

    #[test]
    fn client_reasons_are_the_four_the_protocol_allows() {
        for (word, reason) in [
            ("hangup", EndReason::Hangup),
            ("decline", EndReason::Decline),
            ("cancel", EndReason::Cancel),
            ("failed", EndReason::Failed),
        ] {
            assert_eq!(EndReason::from_client(word), Some(reason));
            assert_eq!(reason.as_str(), word);
        }
        assert_eq!(EndReason::from_client("timeout"), None);
        assert_eq!(EndReason::from_client("answered_elsewhere"), None);
        assert_eq!(EndReason::Timeout.as_str(), "timeout");
        assert_eq!(EndReason::AnsweredElsewhere.as_str(), "answered_elsewhere");
        assert_eq!(Outcome::Missed.placeholder_body(false), "Missed voice call");
        assert_eq!(Outcome::Completed.placeholder_body(false), "Voice call");
        assert_eq!(Outcome::Declined.placeholder_body(false), "Voice call");
        // The video wording is the same old-client courtesy.
        assert_eq!(Outcome::Missed.placeholder_body(true), "Missed video call");
        assert_eq!(Outcome::Completed.placeholder_body(true), "Video call");
        assert_eq!(Outcome::Declined.placeholder_body(true), "Video call");
    }

    /// The flag is fixed at `begin` and rides everything the registry hands
    /// back: the replay to a late callee device (which must ring as a VIDEO
    /// call) and the `Ended` the record is written from.
    #[test]
    fn a_video_call_carries_the_flag_through_replay_and_end() {
        let reg = CallRegistry::new();
        reg.begin(id(1), 42, 7, 9, 100, "offer".to_string(), true, false)
            .expect("begins");
        assert!(reg.pending_offer_for(9).expect("ringing").video);
        let ended = reg.end(id(1), Some(7), EndReason::Cancel).expect("ended");
        assert!(ended.video, "the record must say what kind of call it was");
        // And a voice call stays a voice call.
        reg.begin(id(2), 42, 7, 9, 100, "offer".to_string(), false, false)
            .expect("begins");
        assert!(!reg.pending_offer_for(9).expect("ringing").video);
        let ended = reg.end(id(2), Some(7), EndReason::Cancel).expect("ended");
        assert!(!ended.video);
    }
}
