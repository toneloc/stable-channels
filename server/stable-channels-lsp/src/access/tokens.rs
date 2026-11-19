//! GUI logins are LDK Server tokens: the daemon asks LDK Server who a token is and never reads a token's contents itself.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
use ldk_server_client::ldk_server_grpc::api::{
    CreateMacaroonRequest, GetPermissionsRequest, ListMacaroonsRequest, MacaroonInfo, RevokeMacaroonRequest,
};
use ldk_server_client::macaroon::Macaroon;

use crate::ldk_client::LdkClient;

/// Where LDK Server listens and the certificate it presents, enough to build a client for any token.
pub struct LdkEndpoint {
    pub base_url: String,
    pub cert_pem: Vec<u8>,
}

/// What LDK Server reports about one token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenFacts {
    pub id: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub caveats: Vec<String>,
}

impl From<MacaroonInfo> for TokenFacts {
    fn from(info: MacaroonInfo) -> Self {
        Self { id: info.id, name: info.name, permissions: info.permissions, caveats: info.caveats }
    }
}

#[derive(Debug)]
pub enum TokenCheck {
    Accepted(TokenFacts),
    /// LDK Server answered and said no.
    Refused(String),
    /// LDK Server gave no verdict, so the token is neither accepted nor rejected.
    Unreachable(String),
}

/// A bad or unauthorized token is a refusal; every other failure only means no answer.
pub fn classify(error: &LdkServerError) -> TokenCheck {
    match error.error_code {
        LdkServerErrorCode::AuthError | LdkServerErrorCode::AuthorizationError => {
            TokenCheck::Refused(error.message.clone())
        }
        _ => TokenCheck::Unreachable(error.message.clone()),
    }
}

/// The token's id as LDK Server lists it: the identifier bytes are 32 lowercase hex characters.
pub fn token_root_id(token_hex: &str) -> Option<String> {
    let macaroon = Macaroon::from_hex(token_hex).ok()?;
    let id = std::str::from_utf8(macaroon.identifier()).ok()?;
    let lowercase_hex = id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    lowercase_hex.then(|| id.to_string())
}

/// The daemon can only honor permission and expiry restrictions; a token with any other is refused.
pub fn only_supported_caveats(caveats: &[String]) -> bool {
    caveats.iter().all(|c| c.starts_with("permissions = ") || c.starts_with("time-before = "))
}

#[async_trait]
pub trait TokenAuthority: Send + Sync {
    /// Asks LDK Server who the token is.
    async fn check(&self, token: &str) -> TokenCheck;
    /// Ids of every live token, asked with the daemon's own token.
    async fn live_ids(&self) -> Result<HashSet<String>, LdkServerError>;
    /// Every live token, asked with this token.
    async fn list(&self, token: &str) -> Result<Vec<TokenFacts>, LdkServerError>;
    /// Makes a token with this token; returns the new token's facts and its secret.
    async fn create(
        &self,
        token: &str,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(TokenFacts, String), LdkServerError>;
    /// Revokes the token with this id, asked with this token.
    async fn revoke(&self, token: &str, id: &str) -> Result<(), LdkServerError>;
    /// Makes a token as the daemon's own token, as for the GUI admin token; returns its facts and its secret.
    async fn create_as_daemon(
        &self,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(TokenFacts, String), LdkServerError>;
    /// A client that sends calls as this token; callers keep it with the session.
    fn client(&self, token: &str) -> Result<Arc<LdkClient>, String>;
}

pub struct LdkTokens {
    endpoint: Arc<LdkEndpoint>,
    own: Arc<LdkClient>,
}

impl LdkTokens {
    pub fn new(endpoint: Arc<LdkEndpoint>, own: Arc<LdkClient>) -> Self {
        Self { endpoint, own }
    }
}

fn no_macaroon() -> LdkServerError {
    LdkServerError::new(LdkServerErrorCode::InternalError, "LDK Server's answer carried no token details")
}

/// Asks LDK Server, as this client's token, to make a token; returns its facts and its secret.
async fn create_with(
    client: &LdkClient,
    name: &str,
    permissions: Vec<String>,
) -> Result<(TokenFacts, String), LdkServerError> {
    let response = client.create_macaroon(CreateMacaroonRequest { name: name.to_string(), permissions }).await?;
    let info = response.macaroon.ok_or_else(no_macaroon)?;
    Ok((info.into(), response.token))
}

#[async_trait]
impl TokenAuthority for LdkTokens {
    async fn check(&self, token: &str) -> TokenCheck {
        let client = match self.client(token) {
            Ok(client) => client,
            Err(reason) => return TokenCheck::Refused(reason),
        };
        match client.get_permissions(GetPermissionsRequest {}).await {
            Ok(response) => match response.macaroon {
                Some(info) => TokenCheck::Accepted(info.into()),
                None => classify(&no_macaroon()),
            },
            Err(error) => classify(&error),
        }
    }

    async fn live_ids(&self) -> Result<HashSet<String>, LdkServerError> {
        let response = self.own.list_macaroons(ListMacaroonsRequest {}).await?;
        Ok(response.macaroons.into_iter().map(|m| m.id).collect())
    }

    async fn list(&self, token: &str) -> Result<Vec<TokenFacts>, LdkServerError> {
        let client = self.client(token).map_err(|e| LdkServerError::new(LdkServerErrorCode::AuthError, e))?;
        let response = client.list_macaroons(ListMacaroonsRequest {}).await?;
        Ok(response.macaroons.into_iter().map(TokenFacts::from).collect())
    }

    async fn create(
        &self,
        token: &str,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(TokenFacts, String), LdkServerError> {
        let client = self.client(token).map_err(|e| LdkServerError::new(LdkServerErrorCode::AuthError, e))?;
        create_with(&client, name, permissions).await
    }

    async fn create_as_daemon(
        &self,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(TokenFacts, String), LdkServerError> {
        create_with(&self.own, name, permissions).await
    }

    async fn revoke(&self, token: &str, id: &str) -> Result<(), LdkServerError> {
        let client = self.client(token).map_err(|e| LdkServerError::new(LdkServerErrorCode::AuthError, e))?;
        client.revoke_macaroon(RevokeMacaroonRequest { id: id.to_string() }).await?;
        Ok(())
    }

    fn client(&self, token: &str) -> Result<Arc<LdkClient>, String> {
        let inner = LdkServerClient::new(self.endpoint.base_url.clone(), token.to_string(), &self.endpoint.cert_pem)?;
        Ok(Arc::new(LdkClient::new(inner)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ldk_server_client::macaroon::Macaroon;

    fn error(code: LdkServerErrorCode) -> LdkServerError {
        LdkServerError::new(code, "Macaroon expired")
    }

    fn token_with_id(id: &[u8]) -> String {
        Macaroon::mint(b"root", id).unwrap().to_hex()
    }

    #[test]
    fn refusals_and_outages_are_told_apart() {
        for code in [LdkServerErrorCode::AuthError, LdkServerErrorCode::AuthorizationError] {
            assert!(matches!(classify(&error(code)), TokenCheck::Refused(m) if m == "Macaroon expired"));
        }
        for code in [
            LdkServerErrorCode::InternalError,
            LdkServerErrorCode::InternalServerError,
            LdkServerErrorCode::LightningError,
            LdkServerErrorCode::InvalidRequestError,
        ] {
            assert!(matches!(classify(&error(code)), TokenCheck::Unreachable(_)));
        }
    }

    #[test]
    fn the_root_id_comes_from_the_identifier() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(token_root_id(&token_with_id(id.as_bytes())).as_deref(), Some(id));
        assert_eq!(token_root_id("zz"), None);
        assert_eq!(token_root_id(""), None);
        assert_eq!(token_root_id(&token_with_id(&id.as_bytes()[..31])), None);
        assert_eq!(token_root_id(&token_with_id(b"0123456789ABCDEF0123456789abcdef")), None);
    }

    #[test]
    fn only_permission_and_expiry_restrictions_are_supported() {
        let ok = vec!["permissions = admin".to_string(), "time-before = 1800000000".to_string()];
        assert!(only_supported_caveats(&ok));
        let mut with_method = ok.clone();
        with_method.push("method = GetNodeInfo".to_string());
        assert!(!only_supported_caveats(&with_method));
        let mut with_ip = ok;
        with_ip.push("ip = 1.2.3.4".to_string());
        assert!(!only_supported_caveats(&with_ip));
    }
}
