pub mod apns;
pub mod fcm;
pub mod tokens;

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use tracing::{info, warn};

use crate::config::PushConfig;

const PUSH_COOLDOWN_SECS: u64 = 600; // 10 minutes

pub struct PushService {
    apns: Option<apns::ApnsService>,
    fcm: Option<fcm::FcmService>,
    data_dir: String,
    last_push_sent: HashMap<String, Instant>,
    #[cfg(test)]
    test_backend_available: bool,
}

impl PushService {
    pub fn new(cfg: &PushConfig, data_dir: &Path) -> Self {
        tokens::init_db(data_dir);
        Self {
            apns: apns::ApnsService::try_new(cfg),
            fcm: fcm::FcmService::try_new(cfg),
            data_dir: data_dir.to_string_lossy().to_string(),
            last_push_sent: HashMap::new(),
            #[cfg(test)]
            test_backend_available: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(data_dir: &Path) -> Self {
        let mut service = Self::new(&PushConfig::default(), data_dir);
        service.test_backend_available = true;
        service
    }

    /// Persist a wallet's device token. Called by the RegisterPush handler.
    pub fn register_token(&self, token: &str, platform: &str, node_id: &str, environment: &str) {
        tokens::save_token(&self.data_dir, token, platform, node_id, environment);
    }

    /// Whether enough time has elapsed since the last push to this node.
    pub fn should_notify(&self, node_id: &str) -> bool {
        match self.last_push_sent.get(node_id) {
            Some(last) => last.elapsed().as_secs() >= PUSH_COOLDOWN_SECS,
            None => true,
        }
    }

    fn mark_notified(&mut self, node_id: &str) {
        self.last_push_sent.insert(node_id.to_string(), Instant::now());
    }

    /// Whether a successful wake notification for this node is still within the push cooldown.
    /// This lets another offline channel for the same peer piggyback on the existing wake.
    pub fn has_recent_notification(&self, node_id: &str) -> bool {
        self.last_push_sent
            .get(node_id)
            .is_some_and(|last| last.elapsed().as_secs() < PUSH_COOLDOWN_SECS)
    }

    /// Send a wake notification to the given Lightning node id.
    ///
    /// The result is true only when the configured provider accepts the notification. The
    /// provider call is awaited so a failed request does not consume the node cooldown or start a
    /// wake-settlement poll.
    pub async fn notify(&mut self, node_id: &str, direction: &str) -> bool {
        if !self.should_notify(node_id) {
            info!("[push] Skipping notification for {} (cooldown)", node_id);
            return false;
        }

        let token_info = match tokens::load_token_for_node(&self.data_dir, node_id) {
            Some(t) => t,
            None => {
                warn!("[push] No push token registered for node {}", node_id);
                return false;
            }
        };

        let token = token_info.token;
        let platform = token_info.platform;
        let environment = token_info.environment;
        #[cfg(test)]
        if self.test_backend_available {
            self.mark_notified(node_id);
            return true;
        }
        let sent = if platform == "android" {
            match self.fcm.clone() {
                Some(fcm) => fcm.send(&token, direction, node_id).await,
                None => {
                    warn!(
                        "[push] android platform but FCM disabled, could not send to {}",
                        node_id
                    );
                    false
                }
            }
        } else {
            match self.apns.clone() {
                Some(apns) => apns.send(&token, direction, &environment).await,
                None => {
                    warn!(
                        "[push] ios platform but APNs disabled, could not send to {}",
                        node_id
                    );
                    false
                }
            }
        };

        if sent {
            self.mark_notified(node_id);
            info!(
                "[push] Sent {} notification to {} ({})",
                direction, node_id, platform
            );
        } else {
            warn!(
                "[push] Failed to send {} notification to {}",
                direction, node_id
            );
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn notify_returns_false_without_a_registered_token() {
        let dir = tempdir().unwrap();
        let mut push = PushService::new(&PushConfig::default(), dir.path());

        assert!(!push.notify("missing-node", "lsp_to_user").await);
    }

    #[tokio::test]
    async fn notify_does_not_start_cooldown_when_backend_is_unavailable() {
        let dir = tempdir().unwrap();
        let mut push = PushService::new(&PushConfig::default(), dir.path());
        push.register_token("test-token", "ios", "node", "sandbox");

        assert!(!push.notify("node", "lsp_to_user").await);
        assert!(push.should_notify("node"));
    }

    #[test]
    fn successful_notification_cooldown_is_node_scoped() {
        let dir = tempdir().unwrap();
        let mut push = PushService::new(&PushConfig::default(), dir.path());

        push.mark_notified("node");
        assert!(!push.should_notify("node"));
        assert!(push.has_recent_notification("node"));
        assert!(push.should_notify("other-node"));
    }
}
