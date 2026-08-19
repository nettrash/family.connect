//! Push notification seam.
//!
//! v1 delivers no real push — the protocol only defines the device
//! *registration* hook. Fan-out still computes which offline members would
//! be notified, and hands them to a `PushSender`. Shipping the trait now
//! means APNs/FCM later is a new impl + a config value, with zero changes to
//! the delivery path; the `log` driver also gives integration tests a
//! natural interception point (a recording impl).

use std::sync::Arc;

use async_trait::async_trait;
use tracing::info;

use crate::config::PushConfig;
use crate::models::Message;

/// A device that should be notified about a message: the rows of `devices`
/// with a non-null token belonging to offline chat members.
#[derive(Debug, Clone)]
pub struct DevicePush {
    pub device_id: i64,
    pub user_id: i64,
    pub platform: String,
    pub push_token: String,
}

/// Sink for would-be push notifications. `async_trait` keeps the trait
/// object-safe so `AppState` can hold `Arc<dyn PushSender>`.
#[async_trait]
pub trait PushSender: Send + Sync {
    /// Called once per new message with every offline-member device that has
    /// a push token. Never called for retransmits (dedup hits) and never
    /// includes the sender's own devices.
    async fn notify_new_message(&self, devices: &[DevicePush], message: &Message);
}

/// The `log` driver: records the notification in the server log and does
/// nothing else. Tokens themselves are deliberately not logged — they are
/// credentials for a third-party push service.
pub struct LogPushSender;

#[async_trait]
impl PushSender for LogPushSender {
    async fn notify_new_message(&self, devices: &[DevicePush], message: &Message) {
        for device in devices {
            info!(
                device_id = device.device_id,
                user_id = device.user_id,
                platform = %device.platform,
                chat_id = message.chat_id,
                message_id = message.id,
                "push (log driver): would notify device of new message"
            );
        }
    }
}

/// Build the sender selected by `[push] driver`. Config validation already
/// rejected unknown drivers, so this cannot mis-dispatch at runtime.
pub fn build(cfg: &PushConfig) -> Arc<dyn PushSender> {
    debug_assert_eq!(cfg.driver, "log", "config validation admits only 'log'");
    Arc::new(LogPushSender)
}
