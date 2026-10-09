pub mod apns;
pub mod fcm;
pub mod tokens;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::PushConfig;
use tokens::TokenInfo;

const PUSH_COOLDOWN_SECS: u64 = 600; // 10 minutes
/// A rejected or failed send is retried after this long rather than on every stability tick.
const PUSH_FAILURE_BACKOFF_SECS: u64 = 300;
/// Upper bound on one provider request.
const PUSH_SEND_TIMEOUT_SECS: u64 = 20;

/// Delivers one wake notification to a provider. True only when the provider accepted it.
#[async_trait]
pub(crate) trait PushSender: Send + Sync {
    async fn send(&self, token: &TokenInfo, node_id: &str, direction: &str) -> bool;
}

/// APNs for iOS tokens and FCM for Android tokens; a missing backend rejects the send.
struct ProviderSender {
    apns: Option<apns::ApnsService>,
    fcm: Option<fcm::FcmService>,
}

#[async_trait]
impl PushSender for ProviderSender {
    async fn send(&self, token: &TokenInfo, node_id: &str, direction: &str) -> bool {
        if token.platform == "android" {
            match &self.fcm {
                Some(fcm) => fcm.send(&token.token, direction, node_id).await,
                None => {
                    warn!("[push] android platform but FCM disabled, could not send to {}", node_id);
                    false
                }
            }
        } else {
            match &self.apns {
                Some(apns) => apns.send(&token.token, direction, &token.environment).await,
                None => {
                    warn!("[push] ios platform but APNs disabled, could not send to {}", node_id);
                    false
                }
            }
        }
    }
}

/// A notification reserved by `begin_notify`; the provider call runs without the service lock.
pub(crate) struct PendingPush {
    node_id: String,
    direction: String,
    token: TokenInfo,
    sender: Arc<dyn PushSender>,
}

pub struct PushService {
    sender: Arc<dyn PushSender>,
    data_dir: String,
    last_push_sent: HashMap<String, Instant>,
    /// Sends that failed or have not reported back; a send counts as failed until its provider accepts it.
    last_push_failed: HashMap<String, Instant>,
}

impl PushService {
    pub fn new(cfg: &PushConfig, data_dir: &Path) -> Self {
        let sender = ProviderSender {
            apns: apns::ApnsService::try_new(cfg),
            fcm: fcm::FcmService::try_new(cfg),
        };
        Self::with_sender(Arc::new(sender), data_dir)
    }

    pub(crate) fn with_sender(sender: Arc<dyn PushSender>, data_dir: &Path) -> Self {
        tokens::init_db(data_dir);
        Self {
            sender,
            data_dir: data_dir.to_string_lossy().to_string(),
            last_push_sent: HashMap::new(),
            last_push_failed: HashMap::new(),
        }
    }

    /// Persist a wallet's device token. Called by the RegisterPush handler.
    pub fn register_token(&self, token: &str, platform: &str, node_id: &str, environment: &str) {
        tokens::save_token(&self.data_dir, token, platform, node_id, environment);
    }

    /// Whether a push to this node may be sent now: no accepted push inside the cooldown, and no
    /// failed or still-outstanding one inside the backoff.
    pub fn should_notify(&self, node_id: &str) -> bool {
        let within = |map: &HashMap<String, Instant>, secs: u64| {
            map.get(node_id).is_some_and(|at| at.elapsed().as_secs() < secs)
        };
        !within(&self.last_push_sent, PUSH_COOLDOWN_SECS)
            && !within(&self.last_push_failed, PUSH_FAILURE_BACKOFF_SECS)
    }

    /// Reserve a notification for this node, or None when it is cooling down, already being
    /// sent, or has no registered token.
    pub(crate) fn begin_notify(&mut self, node_id: &str, direction: &str) -> Option<PendingPush> {
        if !self.should_notify(node_id) {
            info!("[push] Skipping notification for {} (cooldown)", node_id);
            return None;
        }
        let Some(token) = tokens::load_token_for_node(&self.data_dir, node_id) else {
            warn!("[push] No push token registered for node {}", node_id);
            return None;
        };
        // Reserved as a failure up front, so a concurrent caller cannot send a duplicate and a send that never reports back still backs off.
        self.last_push_failed.insert(node_id.to_string(), Instant::now());
        Some(PendingPush {
            node_id: node_id.to_string(),
            direction: direction.to_string(),
            token,
            sender: self.sender.clone(),
        })
    }

    /// Record the provider's answer: acceptance starts the cooldown, anything else the backoff.
    pub(crate) fn finish_notify(&mut self, node_id: &str, sent: bool) {
        if sent {
            self.last_push_failed.remove(node_id);
            self.last_push_sent.insert(node_id.to_string(), Instant::now());
        } else {
            self.last_push_failed.insert(node_id.to_string(), Instant::now());
        }
    }
}

impl PendingPush {
    async fn send(&self) -> bool {
        let request = self.sender.send(&self.token, &self.node_id, &self.direction);
        match tokio::time::timeout(Duration::from_secs(PUSH_SEND_TIMEOUT_SECS), request).await {
            Ok(sent) => sent,
            Err(_) => {
                warn!("[push] Provider request for {} timed out", self.node_id);
                false
            }
        }
    }
}

/// Send a wake notification to the given Lightning node id. True only when the provider accepted
/// it. The service lock is released for the provider request, so a slow provider delays neither
/// other nodes' notifications nor token registration.
pub(crate) async fn notify(push: &Arc<Mutex<PushService>>, node_id: &str, direction: &str) -> bool {
    let Some(pending) = push.lock().await.begin_notify(node_id, direction) else {
        return false;
    };
    let sent = pending.send().await;
    push.lock().await.finish_notify(node_id, sent);
    if sent {
        info!("[push] Sent {} notification to {} ({})", direction, node_id, pending.token.platform);
    } else {
        warn!("[push] Failed to send {} notification to {}", direction, node_id);
    }
    sent
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Records every send and answers with a fixed result.
    pub(crate) struct FakeSender {
        pub accept: bool,
        pub sent: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl FakeSender {
        pub(crate) fn new(accept: bool) -> Arc<Self> {
            Arc::new(Self { accept, sent: std::sync::Mutex::new(Vec::new()) })
        }

        pub(crate) fn sent(&self) -> Vec<(String, String)> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PushSender for FakeSender {
        async fn send(&self, _token: &TokenInfo, node_id: &str, direction: &str) -> bool {
            self.sent.lock().unwrap().push((node_id.to_string(), direction.to_string()));
            self.accept
        }
    }

    /// A push service backed by `sender`, with a device token registered for each node.
    pub(crate) fn service(
        sender: Arc<FakeSender>,
        data_dir: &Path,
        nodes: &[&str],
    ) -> Arc<Mutex<PushService>> {
        let service = PushService::with_sender(sender, data_dir);
        for node in nodes {
            service.register_token(&format!("token-{node}"), "ios", node, "sandbox");
        }
        Arc::new(Mutex::new(service))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{service, FakeSender};
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn notify_returns_false_without_a_registered_token() {
        let dir = tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[]);

        assert!(!notify(&push, "missing-node", "lsp_to_user").await);
        assert!(sender.sent().is_empty());
    }

    #[tokio::test]
    async fn rejected_notification_backs_off_without_starting_the_cooldown() {
        let dir = tempdir().unwrap();
        let sender = FakeSender::new(false);
        let push = service(sender.clone(), dir.path(), &["node"]);

        assert!(!notify(&push, "node", "lsp_to_user").await);
        assert!(!notify(&push, "node", "lsp_to_user").await);

        assert_eq!(sender.sent().len(), 1, "a rejected send is not retried inside the backoff");
        assert!(!push.lock().await.last_push_sent.contains_key("node"));
    }

    #[tokio::test]
    async fn accepted_notification_cooldown_is_node_scoped() {
        let dir = tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &["node", "other-node"]);

        assert!(notify(&push, "node", "lsp_to_user").await);
        assert!(!notify(&push, "node", "user_to_lsp").await);
        assert!(notify(&push, "other-node", "user_to_lsp").await);

        assert_eq!(sender.sent().len(), 2);
    }

    #[tokio::test]
    async fn outstanding_request_blocks_a_duplicate_send() {
        let dir = tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &["node"]);

        let mut service = push.lock().await;
        assert!(service.begin_notify("node", "lsp_to_user").is_some());
        assert!(service.begin_notify("node", "lsp_to_user").is_none());
        service.finish_notify("node", true);
        assert!(service.begin_notify("node", "lsp_to_user").is_none(), "cooldown now applies");
    }

    #[tokio::test]
    async fn send_that_never_reports_back_still_backs_off() {
        let dir = tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &["node"]);

        // The task holding this reservation is dropped before it can call finish_notify.
        drop(push.lock().await.begin_notify("node", "lsp_to_user"));

        assert!(!notify(&push, "node", "lsp_to_user").await);
        assert!(sender.sent().is_empty());
    }
}
