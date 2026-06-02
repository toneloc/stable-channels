//! FCM (Android) push sender. try_new returns None when fcm_service_account_path is unset or the file is unreadable.

use serde::Deserialize;
use std::path::Path;
use tracing::{error, info, warn};

use crate::config::PushConfig;

#[derive(Clone)]
pub struct FcmService {
    credentials: FcmCredentials,
}

#[derive(Clone, Deserialize)]
struct FcmCredentials {
    private_key: String,
    client_email: String,
    project_id: String,
}

impl FcmService {
    pub fn try_new(cfg: &PushConfig) -> Option<Self> {
        let path = cfg.fcm_service_account_path.as_deref()?;
        if !Path::new(path).exists() {
            warn!("[fcm] service account file missing at {}. FCM disabled.", path);
            return None;
        }
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("[fcm] failed to read {}: {}. FCM disabled.", path, e);
                return None;
            }
        };
        let credentials: FcmCredentials = match serde_json::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                warn!("[fcm] invalid service account JSON at {}: {}. FCM disabled.", path, e);
                return None;
            }
        };
        info!("[fcm] enabled, service_account_path={}", path);
        Some(Self { credentials })
    }

    pub async fn send(&self, token: &str, direction: &str, node_id: &str) {
        let creds = &self.credentials;

        let access_token = match generate_access_token(creds).await {
            Some(t) => t,
            None => { error!("[fcm] Failed to generate access token"); return; }
        };

        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            creds.project_id
        );

        let body = serde_json::json!({
            "message": {
                "token": token,
                "data": {
                    "stability": serde_json::json!({
                        "direction": direction,
                        "node_id": node_id,
                    }).to_string()
                },
                "android": {
                    "priority": "high"
                }
            }
        });

        match reqwest::Client::new()
            .post(&url)
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    info!("[fcm] Push sent to {}...", &token[..8.min(token.len())]);
                } else {
                    let text = resp.text().await.unwrap_or_default();
                    error!("[fcm] Push failed ({}): {}", status, text);
                }
            }
            Err(e) => error!("[fcm] Request failed: {}", e),
        }
    }
}

async fn generate_access_token(creds: &FcmCredentials) -> Option<String> {
    use openssl::base64;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::sign::Signer;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();

    let header = serde_json::json!({"alg": "RS256", "typ": "JWT"});
    let claims = serde_json::json!({
        "iss": creds.client_email,
        "scope": "https://www.googleapis.com/auth/firebase.messaging",
        "aud": "https://oauth2.googleapis.com/token",
        "iat": now,
        "exp": now + 3600,
    });

    let b64_encode = |data: &[u8]| -> String {
        base64::encode_block(data)
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "")
    };

    let header_b64 = b64_encode(header.to_string().as_bytes());
    let claims_b64 = b64_encode(claims.to_string().as_bytes());
    let signing_input = format!("{}.{}", header_b64, claims_b64);

    let pkey = PKey::private_key_from_pem(creds.private_key.as_bytes()).ok()?;
    let mut signer = Signer::new(MessageDigest::sha256(), &pkey).ok()?;
    signer.update(signing_input.as_bytes()).ok()?;
    let signature = signer.sign_to_vec().ok()?;
    let sig_b64 = b64_encode(&signature);

    let jwt = format!("{}.{}", signing_input, sig_b64);

    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ])
        .send()
        .await
        .ok()?;

    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("access_token")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_new_returns_none_when_path_missing() {
        let cfg = PushConfig::default();
        assert!(FcmService::try_new(&cfg).is_none());
    }

    #[test]
    fn try_new_returns_none_when_file_does_not_exist() {
        let cfg = PushConfig {
            fcm_service_account_path: Some("/nonexistent/firebase.json".to_string()),
            ..Default::default()
        };
        assert!(FcmService::try_new(&cfg).is_none());
    }
}
