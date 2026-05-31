//! APNs (iOS) push sender. try_new returns None (send() then no-ops) when [push] credentials or AuthKey.p8 are missing.

use a2::{
    Client as ApnsClient, ClientConfig as ApnsClientConfig, DefaultNotificationBuilder,
    NotificationBuilder, NotificationOptions, Priority,
};
use std::fs::File;
use tracing::{info, warn};

use crate::config::PushConfig;

#[derive(Clone)]
pub struct ApnsService {
    client: ApnsClient,
    topic: String,
}

impl ApnsService {
    /// Build an APNs client from config. None if a required credential or the key file is missing.
    pub fn try_new(cfg: &PushConfig) -> Option<Self> {
        let key_path = cfg.apns_key_path.as_deref()?;
        let key_id = cfg.apns_key_id.as_deref()?;
        let team_id = cfg.apns_team_id.as_deref()?;
        let topic = cfg.apns_topic.as_deref()?;
        let env = cfg.apns_environment.as_deref().unwrap_or("sandbox");

        let mut key_file = match File::open(key_path) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    "[apns] AuthKey file unreadable at {}: {}. APNs disabled.",
                    key_path, e
                );
                return None;
            }
        };

        let endpoint = if env == "production" {
            a2::Endpoint::Production
        } else {
            a2::Endpoint::Sandbox
        };

        let client = match ApnsClient::token(
            &mut key_file,
            key_id,
            team_id,
            ApnsClientConfig::new(endpoint),
        ) {
            Ok(c) => c,
            Err(e) => {
                warn!("[apns] Failed to construct client: {}. APNs disabled.", e);
                return None;
            }
        };

        info!("[apns] enabled, topic={}, env={}", topic, env);
        Some(Self {
            client,
            topic: topic.to_string(),
        })
    }

    /// Send a wake-up notification to the device token. Direction ("incoming"/"outgoing") rides in the payload.
    pub async fn send(&self, device_token: &str, direction: &str, environment: &str) {
        let _ = environment;
        let body = match direction {
            "lsp_to_user" => "Receiving stability payment...",
            "user_to_lsp" => "Sending stability payment...",
            _ => "Processing payment...",
        };
        let builder = DefaultNotificationBuilder::new()
            .set_title("Stability Update")
            .set_body(body)
            .set_sound("default")
            .set_mutable_content()
            .set_content_available();
        let mut payload = builder.build(
            device_token,
            NotificationOptions {
                apns_topic: Some(&self.topic),
                apns_priority: Some(Priority::High),
                ..Default::default()
            },
        );
        let mut stability_data = std::collections::HashMap::new();
        stability_data.insert("direction", direction);
        let _ = payload.add_custom_data("stability", &stability_data);
        match self.client.send(payload).await {
            Ok(resp) => info!(
                "[apns] Sent push to {} (code={})",
                &device_token[..device_token.len().min(16)],
                resp.code,
            ),
            Err(e) => warn!(
                "[apns] Send failed for {}: {}",
                &device_token[..device_token.len().min(16)],
                e
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_new_returns_none_when_any_field_missing() {
        let cfg = PushConfig::default();
        assert!(ApnsService::try_new(&cfg).is_none());
    }

    #[test]
    fn try_new_returns_none_when_key_file_missing() {
        let cfg = PushConfig {
            apns_key_path: Some("/nonexistent/AuthKey.p8".to_string()),
            apns_key_id: Some("KEY123".to_string()),
            apns_team_id: Some("TEAM123".to_string()),
            apns_topic: Some("com.example.app".to_string()),
            apns_environment: Some("sandbox".to_string()),
            fcm_service_account_path: None,
        };
        assert!(ApnsService::try_new(&cfg).is_none());
    }
}
