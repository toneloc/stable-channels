//! APNs (iOS) push sender. try_new returns None (send() then no-ops) when [push] credentials or AuthKey.p8 are missing.
//!
//! One .p8 signing key is valid for both APNs environments, but sandbox and production
//! are distinct gateways: a token minted by a DEBUG build only resolves at the sandbox
//! endpoint and an App Store build's token only at production. The registered fleet is
//! a mix of both, so we hold a client per endpoint and route each send by the
//! environment the device reported at registration.

use a2::{
    Client as ApnsClient, ClientConfig as ApnsClientConfig, DefaultNotificationBuilder,
    NotificationBuilder, NotificationOptions, Priority,
};
use std::io::Cursor;
use tracing::{info, warn};

use crate::config::PushConfig;

#[derive(Clone)]
pub struct ApnsService {
    sandbox_client: Option<ApnsClient>,
    production_client: Option<ApnsClient>,
    topic: String,
    /// Endpoint used when a stored token has no environment (from `apns_environment`).
    default_environment: String,
}

impl ApnsService {
    /// Build APNs clients from config. None if a required credential or the key file is missing.
    pub fn try_new(cfg: &PushConfig) -> Option<Self> {
        let key_path = cfg.apns_key_path.as_deref()?;
        let key_id = cfg.apns_key_id.as_deref()?;
        let team_id = cfg.apns_team_id.as_deref()?;
        let topic = cfg.apns_topic.as_deref()?;
        let default_environment = cfg
            .apns_environment
            .as_deref()
            .unwrap_or("sandbox")
            .to_string();

        let key_data = match std::fs::read(key_path) {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "[apns] AuthKey file unreadable at {}: {}. APNs disabled.",
                    key_path, e
                );
                return None;
            }
        };

        let build = |endpoint: a2::Endpoint, label: &str| -> Option<ApnsClient> {
            match ApnsClient::token(
                &mut Cursor::new(&key_data),
                key_id,
                team_id,
                ApnsClientConfig::new(endpoint),
            ) {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!("[apns] Failed to construct {} client: {}", label, e);
                    None
                }
            }
        };

        let sandbox_client = build(a2::Endpoint::Sandbox, "sandbox");
        let production_client = build(a2::Endpoint::Production, "production");
        if sandbox_client.is_none() && production_client.is_none() {
            warn!("[apns] No APNs client could be constructed. APNs disabled.");
            return None;
        }

        info!(
            "[apns] enabled, topic={}, default_env={}, sandbox={}, production={}",
            topic,
            default_environment,
            sandbox_client.is_some(),
            production_client.is_some()
        );
        Some(Self {
            sandbox_client,
            production_client,
            topic: topic.to_string(),
            default_environment,
        })
    }

    /// Send a wake-up notification to the device token, routed to the APNs endpoint
    /// matching the environment the token was registered under.
    pub async fn send(&self, device_token: &str, direction: &str, environment: &str) {
        let env = if environment.is_empty() {
            self.default_environment.as_str()
        } else {
            environment
        };
        let client = if env == "production" {
            &self.production_client
        } else {
            &self.sandbox_client
        };
        let client = match client {
            Some(c) => c,
            None => {
                warn!(
                    "[apns] No {} client available; dropping push for {}",
                    env,
                    &device_token[..device_token.len().min(16)]
                );
                return;
            }
        };
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
        match client.send(payload).await {
            Ok(resp) => info!(
                "[apns] Sent push to {} ({}, code={})",
                &device_token[..device_token.len().min(16)],
                env,
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
