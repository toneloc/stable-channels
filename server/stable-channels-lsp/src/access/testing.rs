//! A fake LDK Server and a daemon state that never reaches a real one, shared by the router and token tests.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::Router;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
use ldk_server_client::ldk_server_grpc::error::{ErrorCode, ErrorResponse};
use ldk_server_client::macaroon::Macaroon;
use prost::Message;
use sc_protos::access::{allows, session_header, session_mac, LoginRequest, LoginResponse};
use stable_channels::db::Database;
use tower::ServiceExt;

use crate::access::tokens::{token_root_id, TokenAuthority, TokenCheck, TokenFacts};
use crate::ldk_client::LdkClient;
use crate::state::AppState;

/// The daemon's own token in these tests.
pub const DAEMON_TOKEN_ID: &str = "d43a0c1e5b7f92a6e8c4b0d2f6a81e37";

/// A token-management call the fake LDK Server was asked to make, with the token that made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    List { token: String },
    Create { token: String, name: String, permissions: Vec<String> },
    CreateAsDaemon { name: String, permissions: Vec<String> },
    Revoke { token: String, id: String },
}

/// Answers like LDK Server for the tokens it holds, the daemon's included, and records each token-management call.
pub struct FakeLdk {
    tokens: Mutex<HashMap<String, TokenFacts>>,
    calls: Mutex<Vec<Call>>,
    /// This many `live_ids` calls get no answer before it answers again.
    pub list_outages: AtomicUsize,
    pub live_id_reads: AtomicUsize,
    /// Makes `check` give no answer.
    pub unreachable: AtomicBool,
    /// While held, `revoke` revokes but keeps its answer back.
    pub revoke_answer: tokio::sync::Mutex<()>,
    made: AtomicU64,
    ldk: Arc<LdkClient>,
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn ldk_error(code: LdkServerErrorCode, message: impl Into<String>) -> LdkServerError {
    LdkServerError::new(code, message)
}

impl FakeLdk {
    pub fn new() -> Arc<Self> {
        let fake = Arc::new(Self {
            tokens: Mutex::default(),
            calls: Mutex::default(),
            list_outages: AtomicUsize::new(0),
            live_id_reads: AtomicUsize::new(0),
            unreachable: AtomicBool::new(false),
            revoke_answer: tokio::sync::Mutex::new(()),
            made: AtomicU64::new(0),
            ldk: offline_client_at("localhost:2"),
        });
        fake.set_daemon(&["admin"]);
        fake
    }

    /// A token LDK Server lists and accepts under this name with these permissions.
    pub fn add(&self, n: u128, name: &str, permissions: &[&str]) -> String {
        let id = format!("{:032x}", 0x7a11_0000_0000_0000_0000_0000_0000_0000u128 + n * 0x0102_0304_0506_0708);
        self.insert(id, name, strings(permissions), Vec::new())
    }

    /// Gives the daemon's own token these permissions.
    pub fn set_daemon(&self, permissions: &[&str]) {
        self.insert(DAEMON_TOKEN_ID.to_string(), "admin", strings(permissions), Vec::new());
    }

    /// What LDK Server says about the daemon's own token.
    pub fn daemon(&self) -> TokenFacts {
        self.facts(DAEMON_TOKEN_ID).expect("the daemon token is never revoked in these tests")
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// Drops a token as a revoke with the LDK command line would, without the daemon seeing it.
    pub fn forget(&self, id: &str) {
        self.tokens.lock().unwrap().retain(|_, facts| facts.id != id);
    }

    /// Whether LDK Server still holds a token with this id.
    pub fn holds(&self, id: &str) -> bool {
        self.facts(id).is_some()
    }

    fn facts(&self, id: &str) -> Option<TokenFacts> {
        self.tokens.lock().unwrap().values().find(|facts| facts.id == id).cloned()
    }

    fn insert(&self, id: String, name: &str, permissions: Vec<String>, more_caveats: Vec<String>) -> String {
        let token = test_macaroon(id.as_bytes());
        let mut caveats = vec![format!("permissions = {}", permissions.join(","))];
        caveats.extend(more_caveats);
        let facts = TokenFacts { id, name: name.to_string(), permissions, caveats };
        self.tokens.lock().unwrap().insert(token.clone(), facts);
        token
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    /// The calling token's facts, or LDK Server's refusal when it may not manage tokens.
    fn manager(&self, token: &str) -> Result<TokenFacts, LdkServerError> {
        let facts = token_root_id(token)
            .and_then(|id| self.facts(&id))
            .ok_or_else(|| ldk_error(LdkServerErrorCode::AuthError, "Invalid macaroon credentials"))?;
        if !allows(&facts.permissions, "macaroons:manage") {
            return Err(ldk_error(LdkServerErrorCode::AuthorizationError, "Macaroon management permission required"));
        }
        Ok(facts)
    }

    fn mint(&self, name: &str, permissions: Vec<String>, caveats: Vec<String>) -> Result<(TokenFacts, String), LdkServerError> {
        if self.tokens.lock().unwrap().values().any(|facts| facts.name == name) {
            let taken = format!("macaroon name already exists: {name}");
            return Err(ldk_error(LdkServerErrorCode::InvalidRequestError, taken));
        }
        let n = self.made.fetch_add(1, Ordering::SeqCst);
        let id = format!("{:032x}", 0x6e65_7700_0000_0000_0000_0000_0000_0000u128 + u128::from(n) * 0x0907_0503_0100);
        let token = self.insert(id.clone(), name, permissions, caveats);
        Ok((self.facts(&id).unwrap(), token))
    }
}

#[async_trait]
impl TokenAuthority for FakeLdk {
    async fn check(&self, token: &str) -> TokenCheck {
        if self.unreachable.load(Ordering::SeqCst) {
            return TokenCheck::Unreachable("connection refused".into());
        }
        match self.tokens.lock().unwrap().get(token) {
            Some(facts) => TokenCheck::Accepted(facts.clone()),
            None => TokenCheck::Refused("Invalid macaroon".into()),
        }
    }

    async fn live_ids(&self) -> Result<HashSet<String>, LdkServerError> {
        self.live_id_reads.fetch_add(1, Ordering::SeqCst);
        if self.list_outages.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {
            return Err(ldk_error(LdkServerErrorCode::InternalError, "connection refused"));
        }
        // ListMacaroons needs macaroons:manage, as on a real LDK Server.
        if !allows(&self.daemon().permissions, "macaroons:manage") {
            return Err(ldk_error(LdkServerErrorCode::AuthorizationError, "Macaroon management permission required"));
        }
        Ok(self.tokens.lock().unwrap().values().map(|facts| facts.id.clone()).collect())
    }

    async fn list(&self, token: &str) -> Result<Vec<TokenFacts>, LdkServerError> {
        self.record(Call::List { token: token.to_string() });
        self.manager(token)?;
        let mut all: Vec<TokenFacts> = self.tokens.lock().unwrap().values().cloned().collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(all)
    }

    async fn create(
        &self,
        token: &str,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(TokenFacts, String), LdkServerError> {
        self.record(Call::Create { token: token.to_string(), name: name.to_string(), permissions: permissions.clone() });
        let maker = self.manager(token)?;
        if !allows(&maker.permissions, "admin") && permissions.iter().any(|p| !maker.permissions.contains(p)) {
            return Err(ldk_error(
                LdkServerErrorCode::AuthorizationError,
                "Cannot grant a permission that the calling root does not have",
            ));
        }
        // Like LDK Server, the new token keeps the expiry of the token that made it.
        let expiry = Macaroon::from_hex(token)
            .map(|m| m.caveats().iter().filter_map(|c| String::from_utf8(c.clone()).ok()).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter(|caveat| caveat.starts_with("time-before = "))
            .collect();
        self.mint(name, permissions, expiry)
    }

    async fn create_as_daemon(
        &self,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<(TokenFacts, String), LdkServerError> {
        self.record(Call::CreateAsDaemon { name: name.to_string(), permissions: permissions.clone() });
        if !allows(&self.daemon().permissions, "admin") {
            return Err(ldk_error(
                LdkServerErrorCode::AuthorizationError,
                "Cannot grant a permission that the calling root does not have",
            ));
        }
        self.mint(name, permissions, Vec::new())
    }

    async fn revoke(&self, token: &str, id: &str) -> Result<(), LdkServerError> {
        self.record(Call::Revoke { token: token.to_string(), id: id.to_string() });
        self.manager(token)?;
        {
            let mut tokens = self.tokens.lock().unwrap();
            let before = tokens.len();
            tokens.retain(|_, facts| facts.id != id);
            if tokens.len() == before {
                return Err(ldk_error(LdkServerErrorCode::InvalidRequestError, format!("Unknown macaroon ID: {id}")));
            }
        }
        drop(self.revoke_answer.lock().await);
        Ok(())
    }

    fn client(&self, _: &str) -> Result<Arc<LdkClient>, String> {
        Ok(Arc::clone(&self.ldk))
    }
}

pub fn test_macaroon(id: &[u8]) -> String {
    Macaroon::mint(b"ldk server test root key", id).unwrap().to_hex()
}

/// A client for an LDK Server that is never there, at this address.
pub fn offline_client_at(address: &str) -> Arc<LdkClient> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    crate::tls::get_or_generate_tls_config(None, dir.path().to_str().unwrap()).unwrap();
    let cert = std::fs::read(dir.path().join("tls.crt")).unwrap();
    Arc::new(LdkClient::new(LdkServerClient::new(address.into(), test_macaroon(b"offline"), &cert).unwrap()))
}

/// A state that never reaches LDK Server, with GUI logins open as for an unrestricted admin daemon token.
pub fn offline_state(dir: &Path, fake: &Arc<FakeLdk>) -> AppState {
    let state = offline_state_before_logins(dir, fake);
    open_as(&state, fake, &["admin"]);
    state
}

/// Opens GUI logins as for a daemon token with these permissions.
pub fn open_as(state: &AppState, fake: &FakeLdk, permissions: &[&str]) {
    fake.set_daemon(permissions);
    crate::access::open(state, fake.daemon());
}

/// A state whose GUI logins are not open yet, as before LDK Server first answers about the daemon's token.
pub fn offline_state_before_logins(dir: &Path, fake: &Arc<FakeLdk>) -> AppState {
    let db = Arc::new(Database::open(dir).unwrap());
    AppState {
        ldk_server: offline_client_at("localhost:1"),
        tokens: Arc::clone(fake) as Arc<dyn TokenAuthority>,
        access: Arc::default(),
        gui_token_id: Arc::default(),
        data_dir: dir.to_path_buf(),
        network: "regtest".into(),
        db: Arc::clone(&db),
        push: Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(&Default::default(), dir))),
        push_registrations: Arc::new(crate::push::registrations::Registrations::new(dir)),
        emails: None,
        stable_manager: Arc::new(tokio::sync::Mutex::new(crate::stable_manager::StableChannelManager::new(
            db,
            dir.to_path_buf(),
        ))),
        geoip: crate::geoip::load(None),
        geoip_record_ips: false,
        geoip_retention_secs: 0,
        revenue: Arc::new(crate::revenue::RevenueStore::default()),
        ldk_log_file: None,
        last_event_before_start_ms: None,
        funding_outpoints: Arc::default(),
    }
}

pub async fn post(router: &Router, path: &str, auth: Option<&str>, body: Vec<u8>) -> (StatusCode, Bytes) {
    let mut request = Request::post(path);
    if let Some(auth) = auth {
        request = request.header("x-auth", auth);
    }
    let response = router.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
    let status = response.status();
    (status, axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap())
}

/// The error code and message of a refused request.
pub fn refusal(body: &[u8]) -> (ErrorCode, String) {
    let error = ErrorResponse::decode(body).unwrap();
    (ErrorCode::from_i32(error.error_code).unwrap(), error.message)
}

pub async fn login(router: &Router, token: &str) -> LoginResponse {
    let body = LoginRequest { token: token.to_string() }.encode_to_vec();
    let (status, bytes) = post(router, "/Login", None, body).await;
    assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
    LoginResponse::decode(&bytes[..]).unwrap()
}

pub fn fresh_nonce() -> [u8; 16] {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut nonce = [0x3c; 16];
    nonce[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    nonce
}

/// The X-Auth header the GUI sends for this route and body now.
pub fn sign(login: &LoginResponse, route: &str, body: &[u8]) -> String {
    let secret: [u8; 32] = hex::decode(&login.session_secret).unwrap().try_into().unwrap();
    let ts = crate::access::unix_now();
    let nonce = fresh_nonce();
    session_header(&login.session_id, ts, &nonce, &session_mac(&secret, route, ts, &nonce, body))
}

/// Posts one request to a route, signed with this session.
pub async fn call(router: &Router, session: &LoginResponse, route: &str, body: Vec<u8>) -> (StatusCode, Bytes) {
    post(router, &format!("/{route}"), Some(&sign(session, route, &body)), body).await
}

/// Logs in with a fresh token and posts one signed request to a route.
pub async fn signed_post(router: &Router, token: &str, route: &str, body: Vec<u8>) -> (StatusCode, Bytes) {
    let session = login(router, token).await;
    call(router, &session, route, body).await
}
