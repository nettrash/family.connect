//! ICE on `webrtcbin`: the servers from `GET /calls/ice` (through fc-logic's
//! [`ice_config`](fc_logic::calls::webrtcbin::ice_config), which percent-encodes the TURN user and
//! credential), the candidates both ways, and what ends a call whose far side vanished.
//!
//! **A vanished peer** (app killed, lid shut, network gone) reaches ICE `failed` only if the ICE
//! agent checks that the far side still answers. `webrtcbin`'s libnice agent DOES: on Ubuntu
//! 26.04's GStreamer 1.28.2 / libnice 0.1.23 its `nicesrc`'s agent reports `consent-freshness`,
//! `keepalive-conncheck` and `ice-trickle` all true ([`agent_facts`]), and a peer that vanished
//! mid-call took the survivor's `ice-connection-state` from `completed` straight to `failed` in
//! 24–30 s — frozen (`docker pause`, the lid shut) 29.8 s and 25 s, killed with its network
//! (`docker kill`) 29 s, a Chromium tab's container killed 25 s: RFC 7675's 30-s consent expiry,
//! counted from the last answered check (measured: `tools/call-interop`, cases `g_*`). So
//! the engine's `failed` comes from ICE, as on every other client.
//!
//! The contract's fallback ([`SilenceWatch`]: once connected, no RTP on the far side's audio for
//! [`SILENCE_LIMIT`] → `failed`) is therefore armed only when an agent runs WITHOUT consent
//! freshness ([`SilenceRule::Fallback`], the default — a libnice or GStreamer built otherwise);
//! [`SilenceRule::Always`] arms it on every call (the self-test's and the harness's, to prove it).
//! A muted libwebrtc track still sends Opus silence, a muted GStreamer peer sends encoded silence
//! (the mute is a `volume`, not a valve), and DTX is off unless an SDP says `usedtx=1` — and even
//! DTX sends a frame every 400 ms — so silence on the wire is a dead line, not a quiet person.

use std::time::Duration;

use fc_core::protocol::{IceCandidate, IceServerDto};
use fc_logic::calls::webrtcbin::{IceConfig, ice_config};
use gst::prelude::*;

/// How long the far side's audio may be silent on the wire, once connected, before the call is
/// reported `failed` (`fc_logic::calls`' contract).
pub const SILENCE_LIMIT: Duration = Duration::from_secs(30);

/// Give `webrtcbin` the call's ICE servers: `stun-server` ← the first STUN URL, and one
/// `add-turn-server` per TURN URL. A TURN URI `webrtcbin` refuses is logged and left out — a
/// relay is the last resort, not a reason to fail a call that may connect directly. Never logs a
/// URI: it carries the credential. Returns the config applied (for a test's assertions).
pub fn configure(webrtc: &gst::Element, servers: &[IceServerDto]) -> IceConfig {
    let config = ice_config(servers);
    if let Some(stun) = &config.stun_server {
        webrtc.set_property("stun-server", stun);
    }
    for (index, uri) in config.turn_servers.iter().enumerate() {
        let added = webrtc.emit_by_name::<bool>("add-turn-server", &[uri]);
        if !added {
            glib::g_warning!(
                "family-connect",
                "call: webrtcbin refused TURN server #{index}"
            );
        }
    }
    config
}

/// A candidate `webrtcbin` found (`on-ice-candidate`), as the engine takes it: the index alone —
/// the engine adds the mid from its own offer or answer. `None` for an empty one (never sent).
pub fn gathered(mline: u32, candidate: String) -> Option<IceCandidate> {
    if candidate.trim().is_empty() {
        return None;
    }
    Some(IceCandidate {
        candidate,
        sdp_mid: None,
        sdp_mline_index: i32::try_from(mline).ok(),
    })
}

/// What `add-ice-candidate` takes of a far-side candidate: its m-line index and its text. `None`
/// for the far side's end-of-candidates (an empty candidate — skipped, as the spike did) and for
/// one without a usable index (the engine resolves every index; this is the backstop).
pub fn remote(candidate: &IceCandidate) -> Option<(u32, &str)> {
    if candidate.candidate.trim().is_empty() {
        return None;
    }
    let index = u32::try_from(candidate.sdp_mline_index?).ok()?;
    Some((index, candidate.candidate.as_str()))
}

/// When the dead-peer fallback runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SilenceRule {
    /// Only on a call whose ICE agent runs without consent freshness (or says nothing about it).
    #[default]
    Fallback,
    /// On every call.
    #[allow(dead_code, reason = "tools/call-interop's knob (`--silence-always`)")]
    Always,
}

impl SilenceRule {
    /// Whether to arm the watch for a call whose agent reported `facts` once connected.
    pub fn arms(self, facts: Option<AgentFacts>) -> bool {
        match self {
            SilenceRule::Always => true,
            SilenceRule::Fallback => facts.and_then(|facts| facts.consent_freshness) != Some(true),
        }
    }
}

/// The far side's audio, watched for silence on the wire once the call is connected: fed the
/// count of RTP buffers received so far each tick, it answers `true` once — when the count has
/// not moved for the limit.
#[derive(Debug, Clone)]
pub struct SilenceWatch {
    limit: Duration,
    last_count: Option<u64>,
    since: Duration,
    tripped: bool,
}

impl SilenceWatch {
    pub fn new(limit: Duration) -> Self {
        SilenceWatch {
            limit,
            last_count: None,
            since: Duration::ZERO,
            tripped: false,
        }
    }

    /// `now` is time on any monotonic clock (the watch only subtracts); `count` the far side's
    /// audio RTP buffers so far. `true` once, when the count stood still for the whole limit.
    pub fn observe(&mut self, now: Duration, count: u64) -> bool {
        if self.tripped {
            return false;
        }
        if self.last_count != Some(count) {
            self.last_count = Some(count);
            self.since = now;
            return false;
        }
        if now.saturating_sub(self.since) >= self.limit {
            self.tripped = true;
            return true;
        }
        false
    }
}

/// What the libnice agent under `webrtcbin` was made with — read from a `nicesrc`'s `agent`
/// (`None` before the transport exists, or a property this libnice lacks). Diagnostics: how the
/// dead-peer question was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgentFacts {
    pub consent_freshness: Option<bool>,
    pub keepalive_conncheck: Option<bool>,
    pub ice_trickle: Option<bool>,
}

/// [`AgentFacts`] of the first `nicesrc` inside `webrtc`.
pub fn agent_facts(webrtc: &gst::Element) -> Option<AgentFacts> {
    let bin = webrtc.downcast_ref::<gst::Bin>()?;
    let nicesrc = bin
        .iterate_recurse()
        .into_iter()
        .flatten()
        .find(|element| {
            element
                .factory()
                .is_some_and(|factory| factory.name() == "nicesrc")
        })?;
    let agent = nicesrc.property::<Option<glib::Object>>("agent")?;
    let flag = |name: &str| {
        agent
            .has_property_with_type(name, glib::Type::BOOL)
            .then(|| agent.property::<bool>(name))
    };
    Some(AgentFacts {
        consent_freshness: flag("consent-freshness"),
        keepalive_conncheck: flag("keepalive-conncheck"),
        ice_trickle: flag("ice-trickle"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gathered_candidate_carries_its_index_and_no_mid() {
        assert_eq!(
            gathered(
                0,
                "candidate:1 1 UDP 2015363327 172.18.0.3 41000 typ host".into()
            ),
            Some(IceCandidate {
                candidate: "candidate:1 1 UDP 2015363327 172.18.0.3 41000 typ host".into(),
                sdp_mid: None,
                sdp_mline_index: Some(0),
            })
        );
        assert_eq!(gathered(1, String::new()), None);
        assert_eq!(gathered(1, "  ".into()), None);
    }

    #[test]
    fn a_far_side_candidate_needs_text_and_an_index() {
        let candidate = |text: &str, index: Option<i32>| IceCandidate {
            candidate: text.into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: index,
        };
        assert_eq!(
            remote(&candidate(
                "candidate:2 1 udp 1 10.0.0.2 5000 typ host",
                Some(1)
            )),
            Some((1, "candidate:2 1 udp 1 10.0.0.2 5000 typ host"))
        );
        // End-of-candidates, as libwebrtc and Firefox send it.
        assert_eq!(remote(&candidate("", Some(0))), None);
        assert_eq!(remote(&candidate("candidate:2", None)), None);
        assert_eq!(remote(&candidate("candidate:2", Some(-1))), None);
    }

    #[test]
    fn the_watch_trips_once_after_the_limit_of_standing_still() {
        let mut watch = SilenceWatch::new(Duration::from_secs(30));
        let at = Duration::from_secs;
        assert!(!watch.observe(at(0), 0));
        assert!(!watch.observe(at(29), 0));
        assert!(watch.observe(at(30), 0));
        assert!(!watch.observe(at(31), 0), "once only");
        assert!(!watch.observe(at(90), 0));
    }

    #[test]
    fn any_audio_restarts_the_watch() {
        let mut watch = SilenceWatch::new(Duration::from_secs(30));
        let at = Duration::from_secs;
        assert!(!watch.observe(at(0), 10));
        assert!(!watch.observe(at(25), 10));
        assert!(!watch.observe(at(26), 11));
        assert!(!watch.observe(at(55), 11));
        assert!(watch.observe(at(56), 11));
    }

    /// A call connected before its first far-side packet still counts from the first look.
    #[test]
    fn a_call_that_never_hears_anything_trips_after_the_limit() {
        let mut watch = SilenceWatch::new(Duration::from_secs(30));
        let at = Duration::from_millis;
        assert!(!watch.observe(at(500), 0));
        assert!(!watch.observe(at(30_499), 0));
        assert!(watch.observe(at(30_500), 0));
    }

    #[test]
    fn the_fallback_runs_only_without_consent_freshness() {
        let facts = |consent: Option<bool>| AgentFacts {
            consent_freshness: consent,
            keepalive_conncheck: Some(true),
            ice_trickle: Some(true),
        };
        assert!(!SilenceRule::Fallback.arms(Some(facts(Some(true)))));
        assert!(SilenceRule::Fallback.arms(Some(facts(Some(false)))));
        assert!(SilenceRule::Fallback.arms(Some(facts(None))));
        assert!(SilenceRule::Fallback.arms(None));
        assert!(SilenceRule::Always.arms(Some(facts(Some(true)))));
        assert_eq!(SilenceRule::default(), SilenceRule::Fallback);
    }

    /// A clock read twice in the wrong order (it never should) waits rather than trips.
    #[test]
    fn a_clock_that_went_back_does_not_trip() {
        let mut watch = SilenceWatch::new(Duration::from_secs(30));
        assert!(!watch.observe(Duration::from_secs(100), 4));
        assert!(!watch.observe(Duration::from_secs(50), 4));
    }
}
