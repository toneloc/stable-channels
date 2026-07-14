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
}

impl PushService {
    pub fn new(cfg: &PushConfig, data_dir: &Path) -> Self {
        tokens::init_db(data_dir);
        Self {
            apns: apns::ApnsService::try_new(cfg),
            fcm: fcm::FcmService::try_new(cfg),
            data_dir: data_dir.to_string_lossy().to_string(),
            last_push_sent: HashMap::new(),
        }
    }

    /// Persist a wallet's device token. Called by the RegisterPush handler.
    /// `verified` marks a node-signature-proven registration; verified tokens
    /// win over unsigned ones at notify time (hijack protection, issue #162).
    pub fn register_token(
        &self,
        token: &str,
        platform: &str,
        node_id: &str,
        environment: &str,
        verified: bool,
    ) {
        tokens::save_token(&self.data_dir, token, platform, node_id, environment, verified);
    }

    /// Registration state for a node, so handlers can detect and audit a
    /// token change (potential hijack) before overwriting.
    pub fn node_token_state(&self, node_id: &str) -> tokens::NodeTokenState {
        tokens::node_token_state(&self.data_dir, node_id)
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

    /// Send a wake notification to the given Lightning node id.
    pub fn notify(&mut self, node_id: &str, direction: &str) {
        if !self.should_notify(node_id) {
            info!("[push] Skipping notification for {} (cooldown)", node_id);
            return;
        }

        let token_info = match tokens::load_token_for_node(&self.data_dir, node_id) {
            Some(t) => t,
            None => {
                warn!("[push] No push token registered for node {}", node_id);
                return;
            }
        };

        self.mark_notified(node_id);

        let token = token_info.token.clone();
        let platform = token_info.platform.clone();
        let environment = token_info.environment.clone();
        let node_id_owned = node_id.to_string();
        let direction_owned = direction.to_string();

        if platform == "android" {
            match self.fcm.clone() {
                Some(fcm) => {
                    tokio::spawn(async move {
                        fcm.send(&token, &direction_owned, &node_id_owned).await;
                    });
                }
                None => warn!("[push] android platform but FCM disabled, would have sent to {}", node_id),
            }
        } else {
            match self.apns.clone() {
                Some(apns) => {
                    tokio::spawn(async move {
                        apns.send(&token, &direction_owned, &environment).await;
                    });
                }
                None => warn!("[push] ios platform but APNs disabled, would have sent to {}", node_id),
            }
        }

        info!("[push] Sent {} notification to {} ({})", direction, node_id, platform);
    }
}
