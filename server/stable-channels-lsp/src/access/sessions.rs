//! GUI sessions made from LDK Server tokens: a login makes one, and each GUI request is signed with its secret.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bitcoin_hashes::cmp::fixed_time_eq;
use ldk_server_client::error::LdkServerError;
use ring::rand::{SecureRandom, SystemRandom};
use sc_protos::access::{
    allows, parse_session_header, required, session_mac, LoginResponse, ParsedSession, Requirement, WhoAmIResponse,
};
use tokio::sync::watch;

use crate::access::middleware::CANT_REACH_LDK;
use crate::access::tokens::{classify, only_supported_caveats, token_root_id, TokenAuthority, TokenCheck};
use crate::fixed_window::FixedWindow;
use crate::ldk_client::LdkClient;

/// A session ends this long after its login, and the GUI logs in again.
const SESSION_LIFE_SECS: u64 = 12 * 60 * 60;
/// A further login with the same token ends that token's oldest session.
const MAX_SESSIONS_PER_TOKEN: usize = 8;
/// Past this, new logins are refused rather than ending someone else's session.
const MAX_SESSIONS: usize = 256;
const LOGIN_WINDOW_SECS: u64 = 60;
const LOGINS_PER_WINDOW: u32 = 10;
/// A login naming an unknown token id re-reads LDK Server's token list at most this often.
const LIVE_IDS_REFRESH_SECS: u64 = 60;
/// How far a request's time may be from the daemon's.
const MAX_CLOCK_SKEW_SECS: u64 = 60;
/// A request is stale before its nonce is forgotten, so a nonce can't be used twice.
const NONCE_MEMORY_SECS: u64 = 2 * MAX_CLOCK_SKEW_SECS;
/// A session's token is asked about again once the last answer is this old.
const RECHECK_AFTER_SECS: u64 = 30;
/// While LDK Server gives no answer, the last good one is trusted for this long.
const LAST_ANSWER_LIMIT_SECS: u64 = 120;
/// A session re-check gives up after this long and counts as no answer.
const RECHECK_LIMIT_SECS: u64 = 10;
const MAC_HEX_LEN: usize = 64;

/// Who sent a GUI request; attached to it once every session check passed.
#[derive(Clone)]
pub struct Caller {
    pub session_id: String,
    pub token_id: String,
    /// The person's token, for calls that must go out as them.
    pub token: String,
    /// The token's permissions within the daemon token's cap.
    pub permissions: Vec<String>,
    /// Calls LDK Server with the person's token.
    pub ldk: Arc<LdkClient>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    /// Not a token the daemon can read.
    Malformed,
    /// LDK Server lists no token with this id.
    UnknownId,
    /// Too many logins with this token id in the last minute.
    RateLimited,
    /// The token carries a restriction other than permissions or expiry, such as `method = ...`.
    MethodLimited,
    /// LDK Server refused the token.
    Refused(String),
    /// LDK Server gave no answer.
    Unreachable(String),
    /// The store holds as many sessions as it allows.
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// No well-formed header, or no live session with that id.
    Unknown,
    /// The request's time is too far from the daemon's.
    Stale,
    BadMac,
    Replayed,
    /// LDK Server refused the token at a re-check, so the session is gone.
    Revoked,
    /// LDK Server has given no answer for too long.
    Unreachable,
    Lacks(&'static str),
    /// The route is not open to GUI sessions.
    UnknownRoute,
}

/// What a finished re-check left the session in: usable (fresh, or the last answer kept) or ended.
type Outcome = Result<(), VerifyError>;

/// What a request does about the session's re-check.
enum Step<'a> {
    /// Use the session as it stands.
    Ready,
    /// Ask LDK Server with this token, then tell the requests waiting on this check.
    Check(String, PendingCheck<'a>),
    /// Wait for the check in flight, since the last answer is too old to use.
    Wait(watch::Receiver<Option<Outcome>>),
}

struct Session {
    secret: [u8; 32],
    token: String,
    token_id: String,
    token_name: String,
    permissions: Vec<String>,
    created_at: u64,
    /// Orders logins made in the same second, so "oldest" is exact.
    seq: u64,
    /// When LDK Server last accepted the token.
    checked_at: u64,
    /// The re-check in flight: requests use the last answer meanwhile, or wait for it once that is too old.
    checking: Option<watch::Receiver<Option<Outcome>>>,
    ldk: Arc<LdkClient>,
    /// Each nonce seen and when.
    nonces: HashMap<[u8; 16], u64>,
}

impl Session {
    fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.created_at) >= SESSION_LIFE_SECS
    }

    fn recheck_due(&self, now: u64) -> bool {
        now < self.checked_at || now - self.checked_at >= RECHECK_AFTER_SECS
    }
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    next_seq: u64,
    live_ids: HashSet<String>,
    /// Counts the daemon's own adds and removes, so a list read that began before one is not applied over it.
    live_ids_changes: u64,
    /// Whether the last read of the list got no answer from LDK Server.
    live_ids_unreachable: bool,
    live_ids_read: FixedWindow,
    logins: HashMap<String, FixedWindow>,
}

impl Inner {
    /// The session if it is inside its life; an expired one is removed.
    fn live_session(&mut self, session_id: &str, now: u64) -> Result<&mut Session, VerifyError> {
        if self.sessions.get(session_id).is_some_and(|s| s.expired(now)) {
            self.sessions.remove(session_id);
        }
        self.sessions.get_mut(session_id).ok_or(VerifyError::Unknown)
    }
}

/// In-memory GUI sessions; the lock is never held across a call to LDK Server.
pub struct SessionStore {
    authority: Arc<dyn TokenAuthority>,
    /// The daemon token's permissions when it is not an unrestricted admin; no session gets more.
    cap: Option<Vec<String>>,
    inner: Mutex<Inner>,
}

impl SessionStore {
    pub fn new(authority: Arc<dyn TokenAuthority>, cap: Option<Vec<String>>) -> Self {
        Self { authority, cap, inner: Mutex::default() }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Turns a token into a session; LDK Server is asked only about a listed id within its login budget.
    pub async fn login(&self, token: &str, now: u64) -> Result<LoginResponse, LoginError> {
        let id = token_root_id(token).ok_or(LoginError::Malformed)?;
        let known = self.lock().live_ids.contains(&id);
        if !known {
            self.refresh_live_ids(now).await;
            let inner = self.lock();
            if !inner.live_ids.contains(&id) {
                // Without a list from LDK Server an unknown id may still be a good token, so this is an outage.
                return Err(if inner.live_ids_unreachable {
                    LoginError::Unreachable(CANT_REACH_LDK.to_string())
                } else {
                    LoginError::UnknownId
                });
            }
        }
        {
            let mut inner = self.lock();
            inner.logins.retain(|_, window| window.current(now, LOGIN_WINDOW_SECS));
            if !inner.logins.entry(id).or_default().allow(now, LOGIN_WINDOW_SECS, LOGINS_PER_WINDOW) {
                return Err(LoginError::RateLimited);
            }
        }
        let facts = match self.authority.check(token).await {
            TokenCheck::Accepted(facts) => facts,
            TokenCheck::Refused(why) => return Err(LoginError::Refused(why)),
            TokenCheck::Unreachable(why) => return Err(LoginError::Unreachable(why)),
        };
        if !only_supported_caveats(&facts.caveats) {
            return Err(LoginError::MethodLimited);
        }
        let ldk = self.authority.client(token).map_err(LoginError::Unreachable)?;
        let session_id = hex::encode(random::<16>());
        let secret = random::<32>();
        let permissions = self.effective(&facts.permissions);

        let mut inner = self.lock();
        // A revoke that landed while LDK Server was asked unlisted the id, so no session outlives it.
        if !inner.live_ids.contains(&facts.id) {
            return Err(LoginError::UnknownId);
        }
        inner.sessions.retain(|_, s| !s.expired(now));
        let own: Vec<(u64, String)> = inner
            .sessions
            .iter()
            .filter(|(_, s)| s.token_id == facts.id)
            .map(|(sid, s)| (s.seq, sid.clone()))
            .collect();
        if own.len() >= MAX_SESSIONS_PER_TOKEN {
            if let Some((_, oldest)) = own.into_iter().min() {
                inner.sessions.remove(&oldest);
            }
        } else if inner.sessions.len() >= MAX_SESSIONS {
            return Err(LoginError::Full);
        }
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.sessions.insert(
            session_id.clone(),
            Session {
                secret,
                token: token.to_string(),
                token_id: facts.id.clone(),
                token_name: facts.name.clone(),
                permissions: permissions.clone(),
                created_at: now,
                seq,
                checked_at: now,
                checking: None,
                ldk,
                nonces: HashMap::new(),
            },
        );
        Ok(LoginResponse {
            session_id,
            session_secret: hex::encode(secret),
            token_id: facts.id,
            token_name: facts.name,
            permissions,
            expires_at: now + SESSION_LIFE_SECS,
        })
    }

    /// Checks a signed GUI request: session, time, MAC, nonce, a due re-check, then the route's permission.
    pub async fn verify(
        &self,
        header: Option<&str>,
        route: &str,
        body: &[u8],
        now: u64,
    ) -> Result<Caller, VerifyError> {
        let parsed = header.and_then(parse_session_header).ok_or(VerifyError::Unknown)?;
        {
            let mut inner = self.lock();
            let session = inner.live_session(&parsed.session_id, now)?;
            if now.abs_diff(parsed.ts) > MAX_CLOCK_SKEW_SECS {
                return Err(VerifyError::Stale);
            }
            if !mac_matches(&session.secret, route, &parsed, body) {
                return Err(VerifyError::BadMac);
            }
            session.nonces.retain(|_, seen| now.saturating_sub(*seen) <= NONCE_MEMORY_SECS);
            match session.nonces.entry(parsed.nonce) {
                Entry::Occupied(_) => return Err(VerifyError::Replayed),
                Entry::Vacant(slot) => slot.insert(now),
            };
        }
        loop {
            match self.next_step(&parsed.session_id, now)? {
                Step::Ready => break,
                Step::Check(token, pending) => {
                    self.recheck(&token, pending, now).await?;
                    break;
                }
                Step::Wait(mut in_flight) => {
                    let outcome = in_flight.wait_for(Option::is_some).await.ok().and_then(|outcome| *outcome);
                    match outcome {
                        Some(outcome) => {
                            outcome?;
                            break;
                        }
                        // The checking request was dropped, so waiters ask again: the first makes the next check.
                        None => continue,
                    }
                }
            }
        }

        let inner = self.lock();
        let session = inner.sessions.get(&parsed.session_id).ok_or(VerifyError::Unknown)?;
        if now.saturating_sub(session.checked_at) > LAST_ANSWER_LIMIT_SECS {
            return Err(VerifyError::Unreachable);
        }
        match required(route) {
            Some(Requirement::Permission(permission)) if !allows(&session.permissions, permission) => {
                return Err(VerifyError::Lacks(permission))
            }
            Some(Requirement::Permission(_) | Requirement::AnySession) => {}
            Some(Requirement::NoSession) | None => return Err(VerifyError::UnknownRoute),
        }
        Ok(Caller {
            session_id: parsed.session_id,
            token_id: session.token_id.clone(),
            token: session.token.clone(),
            permissions: session.permissions.clone(),
            ldk: Arc::clone(&session.ldk),
        })
    }

    /// Whether this request uses the session as it stands, makes the due re-check, or waits for the one in flight.
    fn next_step<'a>(&'a self, session_id: &'a str, now: u64) -> Result<Step<'a>, VerifyError> {
        let mut inner = self.lock();
        let session = inner.sessions.get_mut(session_id).ok_or(VerifyError::Unknown)?;
        if !session.recheck_due(now) {
            return Ok(Step::Ready);
        }
        // A check whose maker is gone without answering is no check in flight, so this request makes a new one.
        if session.checking.as_ref().is_some_and(|in_flight| in_flight.has_changed().is_err()) {
            session.checking = None;
        }
        match &session.checking {
            Some(_) if now.saturating_sub(session.checked_at) <= LAST_ANSWER_LIMIT_SECS => Ok(Step::Ready),
            Some(in_flight) => Ok(Step::Wait(in_flight.clone())),
            None => {
                let (done, in_flight) = watch::channel(None);
                session.checking = Some(in_flight);
                let pending = PendingCheck { store: self, session_id, done: Some(done) };
                Ok(Step::Check(session.token.clone(), pending))
            }
        }
    }

    /// Asks LDK Server about the session's token again and tells the requests waiting on it how it ended.
    async fn recheck(&self, token: &str, mut pending: PendingCheck<'_>, now: u64) -> Outcome {
        // Keeps a request's total time under the client's timeout.
        let answer = tokio::time::timeout(Duration::from_secs(RECHECK_LIMIT_SECS), self.authority.check(token))
            .await
            .unwrap_or_else(|_| TokenCheck::Unreachable("LDK Server did not answer the token re-check in time".into()));
        // Applied while the guard still holds `done`, so a panic here clears the check and wakes its waiters.
        let outcome = self.apply(pending.session_id, answer, now);
        if let Some(done) = pending.done.take() {
            done.send_replace(Some(outcome));
        }
        outcome
    }

    /// A refusal ends the session, an acceptance refreshes it, and no answer keeps the last one.
    fn apply(&self, session_id: &str, answer: TokenCheck, now: u64) -> Outcome {
        let mut inner = self.lock();
        if let TokenCheck::Refused(why) = &answer {
            if let Some(session) = inner.sessions.remove(session_id) {
                tracing::info!(token_id = %session.token_id, "GUI session ended, LDK Server refused its token: {why}");
            }
            return Err(VerifyError::Revoked);
        }
        let session = inner.sessions.get_mut(session_id).ok_or(VerifyError::Unknown)?;
        session.checking = None;
        if let TokenCheck::Accepted(facts) = answer {
            session.checked_at = now;
            session.token_name = facts.name;
            session.permissions = self.effective(&facts.permissions);
        }
        Ok(())
    }

    /// The token's permissions, cut down to the daemon token's cap when there is one.
    fn effective(&self, permissions: &[String]) -> Vec<String> {
        let Some(cap) = &self.cap else {
            return permissions.to_vec();
        };
        let in_cap = permissions.iter().filter(|p| allows(cap, p));
        let in_token = cap.iter().filter(|p| allows(permissions, p));
        let mut both: Vec<String> = in_cap.chain(in_token).cloned().collect();
        both.sort();
        both.dedup();
        both
    }

    pub fn logout(&self, session_id: &str) {
        self.lock().sessions.remove(session_id);
    }

    /// Ends every session made from this token, as after a revoke.
    pub fn drop_token(&self, token_id: &str) {
        self.lock().sessions.retain(|_, s| s.token_id != token_id);
    }

    pub fn whoami(&self, session_id: &str) -> Option<WhoAmIResponse> {
        let inner = self.lock();
        let session = inner.sessions.get(session_id)?;
        Some(WhoAmIResponse {
            token_id: session.token_id.clone(),
            token_name: session.token_name.clone(),
            permissions: session.permissions.clone(),
        })
    }

    /// Lists a token the daemon just made, so it can log in before the next read of LDK Server's list.
    pub fn add_live_id(&self, id: &str) {
        let mut inner = self.lock();
        inner.live_ids.insert(id.to_string());
        inner.live_ids_changes += 1;
    }

    /// Unlists a token the daemon just revoked, so a login racing the revoke makes no session.
    pub fn remove_live_id(&self, id: &str) {
        let mut inner = self.lock();
        inner.live_ids.remove(id);
        inner.live_ids_changes += 1;
    }

    /// Reads LDK Server's token list at startup, retrying until it answers; it leaves the login refresh budget alone.
    pub async fn seed_live_ids(&self) {
        let mut retry = Duration::from_secs(1);
        loop {
            match self.read_live_ids().await {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => match classify(&error) {
                    TokenCheck::Refused(why) => {
                        tracing::warn!("LDK Server won't list its tokens to the daemon, so GUI logins can't work: {why}");
                        return;
                    }
                    _ => tracing::warn!(
                        "could not read LDK Server's token list; retrying in {:?}: {}",
                        retry,
                        error.message
                    ),
                },
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(Duration::from_secs(crate::STARTUP_RECONCILE_MAX_RETRY_SECS));
        }
    }

    /// Re-reads LDK Server's live token ids, at most once a minute; a failed read keeps the old list.
    pub async fn refresh_live_ids(&self, now: u64) {
        let due = self.lock().live_ids_read.allow(now, LIVE_IDS_REFRESH_SECS, 1);
        if !due {
            return;
        }
        if let Err(error) = self.read_live_ids().await {
            tracing::warn!("Could not read LDK Server's token list: {}", error.message);
        }
    }

    /// Reads LDK Server's live token ids; false when the daemon's own add or remove during the read made it stale.
    async fn read_live_ids(&self) -> Result<bool, LdkServerError> {
        let changes = self.lock().live_ids_changes;
        let read = self.authority.live_ids().await;
        let mut inner = self.lock();
        inner.live_ids_unreachable = read.as_ref().is_err_and(|e| matches!(classify(e), TokenCheck::Unreachable(_)));
        let ids = read?;
        // Skipped rather than merged: the set already holds those changes, and the next read brings the rest.
        if inner.live_ids_changes != changes {
            return Ok(false);
        }
        inner.live_ids = ids;
        Ok(true)
    }
}

/// If the request is dropped while LDK Server is being asked, clears the session's re-check and wakes its waiters.
struct PendingCheck<'a> {
    store: &'a SessionStore,
    session_id: &'a str,
    /// Taken once LDK Server answers; dropped after `drop` clears the session, so waiters wake to a free slot.
    done: Option<watch::Sender<Option<Outcome>>>,
}

impl Drop for PendingCheck<'_> {
    fn drop(&mut self) {
        if self.done.is_some() {
            if let Some(session) = self.store.lock().sessions.get_mut(self.session_id) {
                session.checking = None;
            }
        }
    }
}

/// Whether the header's MAC is exactly the lowercase hex this session's secret gives for the request.
fn mac_matches(secret: &[u8; 32], route: &str, parsed: &ParsedSession, body: &[u8]) -> bool {
    let lowercase_hex = parsed.mac.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if parsed.mac.len() != MAC_HEX_LEN || !lowercase_hex {
        return false;
    }
    let expected = session_mac(secret, route, parsed.ts, &parsed.nonce, body);
    fixed_time_eq(expected.as_bytes(), parsed.mac.as_bytes())
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new().fill(&mut bytes).expect("the system random source failed");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use ldk_server_client::client::LdkServerClient;
    use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
    use ldk_server_client::ldk_server_grpc::permissions::READONLY_PERMISSIONS;
    use ldk_server_client::macaroon::Macaroon;
    use sc_protos::access::{parse_session_header, session_header, session_mac, LoginResponse};

    use crate::access::tokens::{TokenAuthority, TokenCheck, TokenFacts};
    use crate::ldk_client::LdkClient;

    const NOW: u64 = 1_800_000_000;
    const TWELVE_HOURS: u64 = 12 * 60 * 60;

    /// Answers like LDK Server from a table and counts what it is asked.
    struct FakeAuthority {
        answers: Mutex<HashMap<String, TokenCheck>>,
        /// None makes `live_ids` fail with `list_error`.
        live: Mutex<Option<HashSet<String>>>,
        list_error: Mutex<LdkServerErrorCode>,
        /// How long `live_ids` takes; it answers with the list as it was when asked.
        list_delay: Mutex<Duration>,
        checks: AtomicUsize,
        live_id_reads: AtomicUsize,
        clients: AtomicUsize,
        check_delay: Mutex<Duration>,
        ldk: Arc<LdkClient>,
    }

    fn token_id(n: u32) -> String {
        format!("{:032x}", 0x51ee_c0de_0000_0000_0000_0000_0000_0000u128 + u128::from(n) * 0x0001_0203_0405_0607)
    }

    fn copy(answer: &TokenCheck) -> TokenCheck {
        match answer {
            TokenCheck::Accepted(facts) => TokenCheck::Accepted(facts.clone()),
            TokenCheck::Refused(why) => TokenCheck::Refused(why.clone()),
            TokenCheck::Unreachable(why) => TokenCheck::Unreachable(why.clone()),
        }
    }

    /// A client for a daemon that never reaches LDK Server; sessions only carry it.
    fn offline_client() -> Arc<LdkClient> {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        crate::tls::get_or_generate_tls_config(None, dir.path().to_str().unwrap()).unwrap();
        let cert = std::fs::read(dir.path().join("tls.crt")).unwrap();
        let token = Macaroon::mint(b"session test root key", b"offline").unwrap().to_hex();
        Arc::new(LdkClient::new(LdkServerClient::new("localhost:1".into(), token, &cert).unwrap()))
    }

    impl FakeAuthority {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::default(),
                live: Mutex::new(Some(HashSet::new())),
                list_error: Mutex::new(LdkServerErrorCode::InternalError),
                list_delay: Mutex::new(Duration::ZERO),
                checks: AtomicUsize::new(0),
                live_id_reads: AtomicUsize::new(0),
                clients: AtomicUsize::new(0),
                check_delay: Mutex::new(Duration::ZERO),
                ldk: offline_client(),
            })
        }

        /// A token LDK Server lists and accepts with these permissions.
        fn add(&self, n: u32, permissions: &[&str]) -> String {
            let id = token_id(n);
            let token = Macaroon::mint(b"session test root key", id.as_bytes()).unwrap().to_hex();
            let permissions: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
            let caveats = vec![format!("permissions = {}", permissions.join(","))];
            let facts = TokenFacts { id: id.clone(), name: format!("operator-{n}"), permissions, caveats };
            self.answer(&token, TokenCheck::Accepted(facts));
            self.live.lock().unwrap().get_or_insert_with(HashSet::new).insert(id);
            token
        }

        fn answer(&self, token: &str, answer: TokenCheck) {
            self.answers.lock().unwrap().insert(token.to_string(), answer);
        }

        fn checks(&self) -> usize {
            self.checks.load(Ordering::SeqCst)
        }

        fn live_id_reads(&self) -> usize {
            self.live_id_reads.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl TokenAuthority for FakeAuthority {
        async fn check(&self, token: &str) -> TokenCheck {
            self.checks.fetch_add(1, Ordering::SeqCst);
            let delay = *self.check_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            match self.answers.lock().unwrap().get(token) {
                Some(answer) => copy(answer),
                None => TokenCheck::Refused("unknown token".into()),
            }
        }

        async fn live_ids(&self) -> Result<HashSet<String>, LdkServerError> {
            self.live_id_reads.fetch_add(1, Ordering::SeqCst);
            let live = self.live.lock().unwrap().clone();
            let delay = *self.list_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            live.ok_or_else(|| LdkServerError::new(self.list_error.lock().unwrap().clone(), "LDK Server is down"))
        }

        async fn list(&self, _: &str) -> Result<Vec<TokenFacts>, LdkServerError> {
            unreachable!("sessions never list tokens")
        }

        async fn create(&self, _: &str, _: &str, _: Vec<String>) -> Result<(TokenFacts, String), LdkServerError> {
            unreachable!("sessions never create tokens")
        }

        async fn revoke(&self, _: &str, _: &str) -> Result<(), LdkServerError> {
            unreachable!("sessions never revoke tokens")
        }

        async fn create_as_daemon(&self, _: &str, _: Vec<String>) -> Result<(TokenFacts, String), LdkServerError> {
            unreachable!("sessions never create tokens")
        }

        fn client(&self, _: &str) -> Result<Arc<LdkClient>, String> {
            self.clients.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::clone(&self.ldk))
        }
    }

    fn store(fake: &Arc<FakeAuthority>, cap: Option<&[&str]>) -> SessionStore {
        let cap = cap.map(|c| c.iter().map(|p| p.to_string()).collect());
        SessionStore::new(Arc::clone(fake) as Arc<dyn TokenAuthority>, cap)
    }

    fn session_count(store: &SessionStore) -> usize {
        store.lock().sessions.len()
    }

    fn nonce_count(store: &SessionStore, session_id: &str) -> usize {
        store.lock().sessions[session_id].nonces.len()
    }

    fn fresh_nonce() -> [u8; 16] {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut nonce = [0x6e; 16];
        nonce[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        nonce
    }

    /// The header the GUI sends for this request.
    fn signed(login: &LoginResponse, route: &str, body: &[u8], ts: u64, nonce: [u8; 16]) -> String {
        let secret: [u8; 32] = hex::decode(&login.session_secret).unwrap().try_into().unwrap();
        session_header(&login.session_id, ts, &nonce, &session_mac(&secret, route, ts, &nonce, body))
    }

    /// A correctly signed empty-body request sent at `now`.
    async fn call(store: &SessionStore, login: &LoginResponse, route: &str, now: u64) -> Result<Caller, VerifyError> {
        let header = signed(login, route, b"", now, fresh_nonce());
        store.verify(Some(&header), route, b"", now).await
    }

    #[tokio::test]
    async fn a_good_token_logs_in_and_signed_requests_pass() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);

        let login = store.login(&token, NOW).await.unwrap();
        assert_eq!(login.token_id, token_id(1));
        assert_eq!(login.token_name, "operator-1");
        assert_eq!(login.permissions, ["admin"]);
        assert_eq!(login.expires_at, NOW + TWELVE_HOURS);
        assert_eq!(hex::decode(&login.session_id).unwrap().len(), 16);
        assert_eq!(hex::decode(&login.session_secret).unwrap().len(), 32);

        let body = br#"{"channel_id":"ab12"}"#;
        let header = signed(&login, "EditStableChannel", body, NOW + 1, fresh_nonce());
        let caller = store.verify(Some(&header), "EditStableChannel", body, NOW + 1).await.unwrap();
        assert_eq!(caller.session_id, login.session_id);
        assert_eq!(caller.token_id, token_id(1));
        assert_eq!(caller.token, token);
        assert_eq!(caller.permissions, ["admin"]);
        assert!(Arc::ptr_eq(&caller.ldk, &fake.ldk), "calls go out through the client made for this token");
        assert!(call(&store, &login, "GetNodeInfo", NOW + 29).await.is_ok());
        assert_eq!(fake.checks(), 1, "requests inside 30 s use the login's answer");
        assert_eq!(fake.clients.load(Ordering::SeqCst), 1, "one LDK client per session");

        let again = store.login(&token, NOW).await.unwrap();
        assert_ne!(again.session_id, login.session_id);
        assert_ne!(again.session_secret, login.session_secret);
    }

    #[tokio::test]
    async fn a_malformed_token_or_unknown_id_is_refused_without_asking_ldk() {
        let fake = FakeAuthority::new();
        fake.add(1, &["admin"]);
        let store = store(&fake, None);

        let short_id = Macaroon::mint(b"session test root key", b"0123").unwrap().to_hex();
        for bad in ["", "zz", "not a token", &"ab".repeat(40), &short_id] {
            assert_eq!(store.login(bad, NOW).await.err(), Some(LoginError::Malformed), "{bad}");
        }
        let stranger = Macaroon::mint(b"some other root key", token_id(99).as_bytes()).unwrap().to_hex();
        assert_eq!(store.login(&stranger, NOW).await.err(), Some(LoginError::UnknownId));
        assert_eq!(store.login(&stranger, NOW + 1).await.err(), Some(LoginError::UnknownId));
        assert_eq!(fake.checks(), 0);
        assert_eq!(fake.live_id_reads(), 1, "unknown ids re-read the token list at most once a minute");
        assert_eq!(session_count(&store), 0);
    }

    #[tokio::test]
    async fn a_method_limited_admin_token_cannot_log_in() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let caveats = vec!["permissions = admin".to_string(), "method = GetNodeInfo".to_string()];
        let permissions = vec!["admin".to_string()];
        let facts = TokenFacts { id: token_id(1), name: "operator-1".into(), permissions, caveats };
        fake.answer(&token, TokenCheck::Accepted(facts));
        let store = store(&fake, None);

        assert_eq!(store.login(&token, NOW).await.err(), Some(LoginError::MethodLimited));
        assert_eq!(session_count(&store), 0);
    }

    #[tokio::test]
    async fn an_expired_token_is_refused_at_login_and_loses_its_session_at_the_recheck() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        fake.answer(&token, TokenCheck::Refused("Macaroon expired".into()));
        assert_eq!(store.login(&token, NOW + 1).await.err(), Some(LoginError::Refused("Macaroon expired".into())));
        assert!(call(&store, &login, "GetNodeInfo", NOW + 29).await.is_ok(), "the last answer holds for 30 s");
        assert_eq!(call(&store, &login, "GetNodeInfo", NOW + 30).await.err(), Some(VerifyError::Revoked));
        assert_eq!(call(&store, &login, "GetNodeInfo", NOW + 31).await.err(), Some(VerifyError::Unknown));
        assert_eq!(session_count(&store), 0);
        assert_eq!(fake.checks(), 3);
    }

    #[tokio::test]
    async fn an_unreachable_ldk_keeps_the_last_answer_for_two_minutes_then_refuses() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let accepted = copy(&fake.answers.lock().unwrap()[&token]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        fake.answer(&token, TokenCheck::Unreachable("connection refused".into()));
        let unreachable = Some(LoginError::Unreachable("connection refused".into()));
        assert_eq!(store.login(&token, NOW + 1).await.err(), unreachable);
        assert!(call(&store, &login, "GetNodeInfo", NOW + 30).await.is_ok());
        assert!(call(&store, &login, "GetNodeInfo", NOW + 120).await.is_ok());
        assert_eq!(call(&store, &login, "GetNodeInfo", NOW + 121).await.err(), Some(VerifyError::Unreachable));

        fake.answer(&token, accepted);
        assert!(call(&store, &login, "GetNodeInfo", NOW + 122).await.is_ok(), "the session was kept");
        assert!(call(&store, &login, "GetNodeInfo", NOW + 123).await.is_ok());
        assert_eq!(fake.checks(), 6, "two logins, then re-checks at +30, +120, +121 and +122");
    }

    #[tokio::test]
    async fn stale_bad_mac_replayed_and_unknown_session_are_refused() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();
        let verify = |header: String, route: &'static str, body: &'static [u8]| {
            let store = &store;
            async move { store.verify(Some(&header), route, body, NOW).await.err() }
        };

        for (ts, answer) in [(NOW - 61, Some(VerifyError::Stale)), (NOW + 61, Some(VerifyError::Stale)), (NOW - 60, None)] {
            let header = signed(&login, "GetNodeInfo", b"", ts, fresh_nonce());
            assert_eq!(verify(header, "GetNodeInfo", b"").await, answer, "signed at {ts}");
        }

        let header = signed(&login, "GetNodeInfo", b"a", NOW, fresh_nonce());
        assert_eq!(verify(header.clone(), "GetNodeInfo", b"b").await, Some(VerifyError::BadMac));
        assert_eq!(verify(header.clone(), "GetBalances", b"a").await, Some(VerifyError::BadMac));
        let parsed = parse_session_header(&header).unwrap();
        for mac in [parsed.mac.to_uppercase(), parsed.mac[..63].to_string(), format!("{}0", parsed.mac)] {
            let odd = session_header(&parsed.session_id, parsed.ts, &parsed.nonce, &mac);
            assert_eq!(verify(odd, "GetNodeInfo", b"a").await, Some(VerifyError::BadMac), "{mac}");
        }
        let first = verify(header.clone(), "GetNodeInfo", b"a").await;
        assert_eq!(first, None, "a refused request does not use up its nonce");
        assert_eq!(verify(header, "GetNodeInfo", b"a").await, Some(VerifyError::Replayed));

        let stranger = LoginResponse { session_id: "5e".repeat(16), ..login.clone() };
        let header = signed(&stranger, "GetNodeInfo", b"", NOW, fresh_nonce());
        assert_eq!(verify(header, "GetNodeInfo", b"").await, Some(VerifyError::Unknown));
        assert_eq!(verify("HMAC 1800000000:ab".into(), "GetNodeInfo", b"").await, Some(VerifyError::Unknown));
        assert_eq!(store.verify(None, "GetNodeInfo", b"", NOW).await.err(), Some(VerifyError::Unknown));
    }

    #[tokio::test]
    async fn a_read_only_session_lacks_money_and_admin_routes() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &READONLY_PERMISSIONS);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        for (route, lacks) in
            [("ReleaseStabilityPayment", "payments:send"), ("LdkLog", "admin"), ("CreateToken", "macaroons:manage")]
        {
            assert_eq!(call(&store, &login, route, NOW).await.err(), Some(VerifyError::Lacks(lacks)), "{route}");
        }
        assert!(call(&store, &login, "ListStableChannels", NOW).await.is_ok());
    }

    #[tokio::test]
    async fn the_daemon_token_cap_limits_admin_sessions() {
        let fake = FakeAuthority::new();
        let admin = fake.add(1, &["admin"]);
        let reader = fake.add(2, &["payments:read", "node:read"]);
        let store = store(&fake, Some(&["node:read"]));

        let login = store.login(&admin, NOW).await.unwrap();
        assert_eq!(login.permissions, ["node:read"]);
        let edit = call(&store, &login, "EditStableChannel", NOW).await;
        assert_eq!(edit.err(), Some(VerifyError::Lacks("channels:manage")));
        let caller = call(&store, &login, "GetNodeInfo", NOW).await.unwrap();
        assert_eq!(caller.permissions, ["node:read"]);
        assert_eq!(store.whoami(&login.session_id).unwrap().permissions, ["node:read"]);

        let narrower = store.login(&reader, NOW).await.unwrap();
        assert_eq!(narrower.permissions, ["node:read"], "only what both the token and the cap hold");
        let after_recheck = call(&store, &login, "GetNodeInfo", NOW + 30).await.unwrap();
        assert_eq!(after_recheck.permissions, ["node:read"], "a re-check keeps the cap");
    }

    #[tokio::test]
    async fn ten_logins_a_minute_per_token_then_rate_limited() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let other = fake.add(2, &["admin"]);
        let refused = fake.add(3, &["admin"]);
        fake.answer(&refused, TokenCheck::Refused("Macaroon expired".into()));
        let store = store(&fake, None);

        for i in 0..10 {
            assert!(store.login(&token, NOW + i).await.is_ok(), "login {i}");
        }
        assert_eq!(store.login(&token, NOW + 59).await.err(), Some(LoginError::RateLimited));
        assert_eq!(fake.checks(), 10, "a limited login does not ask LDK Server");
        assert!(store.login(&other, NOW + 59).await.is_ok(), "the limit is per token");
        assert!(store.login(&token, NOW + 60).await.is_ok(), "a new minute");

        for _ in 0..10 {
            assert!(matches!(store.login(&refused, NOW).await, Err(LoginError::Refused(_))));
        }
        assert_eq!(store.login(&refused, NOW).await.err(), Some(LoginError::RateLimited), "refused attempts count too");
        assert_eq!(fake.checks(), 22);
    }

    #[tokio::test]
    async fn a_ninth_session_for_one_token_drops_its_oldest_and_a_full_store_refuses_new_logins() {
        let fake = FakeAuthority::new();
        let tokens: Vec<String> = (0..33).map(|n| fake.add(n, &["node:read"])).collect();
        let store = store(&fake, None);

        let mut first = Vec::new();
        for _ in 0..9 {
            first.push(store.login(&tokens[0], NOW).await.unwrap());
        }
        let oldest = call(&store, &first[0], "GetNodeInfo", NOW).await;
        assert_eq!(oldest.err(), Some(VerifyError::Unknown), "the oldest is dropped");
        for login in &first[1..] {
            assert!(call(&store, login, "GetNodeInfo", NOW).await.is_ok());
        }

        let mut others = Vec::new();
        for token in &tokens[1..32] {
            for _ in 0..8 {
                others.push(store.login(token, NOW).await.unwrap());
            }
        }
        assert_eq!(session_count(&store), 256);
        assert_eq!(store.login(&tokens[32], NOW).await.err(), Some(LoginError::Full));
        assert!(store.login(&tokens[0], NOW).await.is_ok(), "a token at its own limit replaces its own oldest");
        assert_eq!(call(&store, &first[1], "GetNodeInfo", NOW).await.err(), Some(VerifyError::Unknown));
        assert_eq!(session_count(&store), 256);
        for login in &others {
            assert!(call(&store, login, "GetNodeInfo", NOW).await.is_ok(), "nobody else's session is dropped");
        }

        assert!(store.login(&tokens[32], NOW + TWELVE_HOURS).await.is_ok(), "sessions past their life free room");
        assert_eq!(session_count(&store), 1);
    }

    #[tokio::test]
    async fn concurrent_requests_due_for_a_recheck_ask_ldk_once() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        let login = store.login(&token, NOW).await.unwrap();
        *fake.check_delay.lock().unwrap() = Duration::from_millis(50);

        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let store = Arc::clone(&store);
            let header = signed(&login, "GetNodeInfo", b"", NOW + 30, fresh_nonce());
            requests.spawn(async move { store.verify(Some(&header), "GetNodeInfo", b"", NOW + 30).await.is_ok() });
        }
        while let Some(ok) = requests.join_next().await {
            assert!(ok.unwrap());
        }
        assert_eq!(fake.checks(), 2, "the login and one re-check");
    }

    #[tokio::test]
    async fn a_cancelled_recheck_does_not_block_later_ones() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        *fake.check_delay.lock().unwrap() = Duration::from_secs(3600);
        let header = signed(&login, "GetNodeInfo", b"", NOW + 30, fresh_nonce());
        let request = store.verify(Some(&header), "GetNodeInfo", b"", NOW + 30);
        assert!(tokio::time::timeout(Duration::from_millis(20), request).await.is_err(), "the client went away mid re-check");

        *fake.check_delay.lock().unwrap() = Duration::ZERO;
        assert!(call(&store, &login, "GetNodeInfo", NOW + 31).await.is_ok());
        assert_eq!(fake.checks(), 3, "the next request asks again");
        assert!(call(&store, &login, "GetNodeInfo", NOW + 150).await.is_ok());
    }

    /// A request's caller permissions, or why it was refused.
    type Answer = Result<Vec<String>, VerifyError>;

    /// A signed GetNodeInfo sent at `now` on its own task.
    fn spawn_request(store: &Arc<SessionStore>, login: &LoginResponse, now: u64) -> tokio::task::JoinHandle<Answer> {
        let store = Arc::clone(store);
        let header = signed(login, "GetNodeInfo", b"", now, fresh_nonce());
        tokio::spawn(async move { store.verify(Some(&header), "GetNodeInfo", b"", now).await.map(|c| c.permissions) })
    }

    /// The request's answer, failing the test if it never comes.
    async fn answer_of(request: tokio::task::JoinHandle<Answer>) -> Answer {
        tokio::time::timeout(Duration::from_secs(5), request).await.expect("the request is answered").unwrap()
    }

    /// 20 requests at once on a session idle for 200 s, while LDK Server takes 50 ms to give `answer`.
    async fn idle_burst(answer: TokenCheck) -> (Arc<FakeAuthority>, Arc<SessionStore>, LoginResponse, Vec<Answer>) {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        let login = store.login(&token, NOW).await.unwrap();
        fake.answer(&token, answer);
        *fake.check_delay.lock().unwrap() = Duration::from_millis(50);

        let requests: Vec<_> = (0..20).map(|_| spawn_request(&store, &login, NOW + 200)).collect();
        let mut answers = Vec::new();
        for request in requests {
            answers.push(answer_of(request).await);
        }
        (fake, store, login, answers)
    }

    #[tokio::test]
    async fn an_idle_session_burst_waits_for_the_one_recheck() {
        let (permissions, caveats) = (vec!["node:read".to_string()], vec!["permissions = node:read".to_string()]);
        let facts = TokenFacts { id: token_id(1), name: "operator-1".into(), permissions, caveats };
        let (fake, _, _, answers) = idle_burst(TokenCheck::Accepted(facts)).await;
        assert_eq!(answers, vec![Ok(vec!["node:read".to_string()]); 20], "every request uses the fresh answer");
        assert_eq!(fake.checks(), 2, "the login and one re-check");
    }

    #[tokio::test]
    async fn an_idle_session_burst_shares_a_refusal() {
        let (fake, store, login, answers) = idle_burst(TokenCheck::Refused("Macaroon revoked".into())).await;
        assert_eq!(answers, vec![Err(VerifyError::Revoked); 20]);
        assert_eq!(fake.checks(), 2);
        assert!(store.whoami(&login.session_id).is_none(), "the session is gone");
    }

    #[tokio::test]
    async fn an_idle_session_burst_shares_an_outage() {
        let (fake, store, login, answers) = idle_burst(TokenCheck::Unreachable("connection refused".into())).await;
        assert_eq!(answers, vec![Err(VerifyError::Unreachable); 20]);
        assert_eq!(fake.checks(), 2);
        assert!(store.whoami(&login.session_id).is_some(), "the session is kept for when LDK Server is back");
    }

    #[tokio::test]
    async fn a_cancelled_check_does_not_leave_its_waiters_hanging() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        let login = store.login(&token, NOW).await.unwrap();
        *fake.check_delay.lock().unwrap() = Duration::from_secs(3600);

        let checking = spawn_request(&store, &login, NOW + 200);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let waiting: Vec<_> = (0..5).map(|_| spawn_request(&store, &login, NOW + 200)).collect();
        tokio::time::sleep(Duration::from_millis(20)).await;
        *fake.check_delay.lock().unwrap() = Duration::ZERO;
        checking.abort();
        assert!(checking.await.unwrap_err().is_cancelled());

        for request in waiting {
            assert!(answer_of(request).await.is_ok(), "a waiter asks again itself");
        }
        assert_eq!(fake.checks(), 3, "the login, the cancelled check and one check by a waiter");
    }

    #[tokio::test]
    async fn requests_waiting_on_a_check_learn_that_the_session_ended() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        let login = store.login(&token, NOW).await.unwrap();
        *fake.check_delay.lock().unwrap() = Duration::from_millis(100);

        let checking = spawn_request(&store, &login, NOW + 200);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let waiting: Vec<_> = (0..5).map(|_| spawn_request(&store, &login, NOW + 200)).collect();
        tokio::time::sleep(Duration::from_millis(20)).await;
        store.logout(&login.session_id);

        assert_eq!(answer_of(checking).await, Err(VerifyError::Unknown));
        for request in waiting {
            assert_eq!(answer_of(request).await, Err(VerifyError::Unknown));
        }
        assert_eq!(fake.checks(), 2);
    }

    #[tokio::test]
    async fn requests_within_two_minutes_do_not_wait_for_a_slow_recheck() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        let login = store.login(&token, NOW).await.unwrap();
        *fake.check_delay.lock().unwrap() = Duration::from_secs(3600);

        let checking = spawn_request(&store, &login, NOW + 30);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let quick = tokio::time::timeout(Duration::from_secs(1), call(&store, &login, "GetNodeInfo", NOW + 31)).await;
        assert!(quick.expect("the last answer is used, not waited on").is_ok());
        assert_eq!(fake.checks(), 2);
        checking.abort();
    }

    /// Virtual seconds a verify takes at `now` while the re-check never answers, and its result.
    async fn verify_with_hanging_check(now: u64, fake: &Arc<FakeAuthority>, store: &SessionStore, login: &LoginResponse) -> (u64, Result<Caller, VerifyError>) {
        *fake.check_delay.lock().unwrap() = Duration::from_secs(3600);
        let started = tokio::time::Instant::now();
        let result = call(store, login, "GetNodeInfo", now).await;
        (started.elapsed().as_secs(), result)
    }

    #[tokio::test(start_paused = true)]
    async fn a_recheck_that_hangs_gives_up_after_ten_seconds_and_keeps_the_last_answer() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        let (took, result) = verify_with_hanging_check(NOW + 30, &fake, &store, &login).await;
        assert!(result.is_ok(), "the last answer is still within two minutes");
        assert!((10..=11).contains(&took), "the re-check gave up after {took} s");
        assert_eq!(fake.checks(), 2, "the login and one re-check");
        assert!(store.whoami(&login.session_id).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_recheck_that_hangs_past_the_grace_refuses() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        let (took, result) = verify_with_hanging_check(NOW + 130, &fake, &store, &login).await;
        assert_eq!(result.err(), Some(VerifyError::Unreachable));
        assert!((10..=11).contains(&took), "the re-check gave up after {took} s");
        assert_eq!(fake.checks(), 2);
        assert!(store.whoami(&login.session_id).is_some(), "the session is kept for when LDK Server is back");
    }

    #[tokio::test]
    async fn the_nonce_memory_forgets_entries_older_than_two_minutes() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        for now in [NOW, NOW, NOW + 60] {
            call(&store, &login, "GetNodeInfo", now).await.unwrap();
        }
        assert_eq!(nonce_count(&store, &login.session_id), 3);
        call(&store, &login, "GetNodeInfo", NOW + 120).await.unwrap();
        assert_eq!(nonce_count(&store, &login.session_id), 4, "two minutes old is still remembered");
        call(&store, &login, "GetNodeInfo", NOW + 121).await.unwrap();
        assert_eq!(nonce_count(&store, &login.session_id), 3, "the two from NOW are forgotten");
    }

    #[tokio::test]
    async fn drop_token_ends_every_session_of_that_token() {
        let fake = FakeAuthority::new();
        let revoked = fake.add(1, &["admin"]);
        let kept = fake.add(2, &["admin"]);
        let store = store(&fake, None);
        let gone = [store.login(&revoked, NOW).await.unwrap(), store.login(&revoked, NOW).await.unwrap()];
        let other = store.login(&kept, NOW).await.unwrap();

        store.drop_token(&token_id(1));
        for login in &gone {
            assert_eq!(call(&store, login, "GetNodeInfo", NOW).await.err(), Some(VerifyError::Unknown));
            assert!(store.whoami(&login.session_id).is_none());
        }
        assert!(call(&store, &other, "GetNodeInfo", NOW).await.is_ok());
    }

    #[tokio::test]
    async fn login_and_unlisted_routes_are_not_open_to_sessions() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["node:read"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        assert_eq!(call(&store, &login, "Login", NOW).await.err(), Some(VerifyError::UnknownRoute));
        assert_eq!(call(&store, &login, "NoSuchRoute", NOW).await.err(), Some(VerifyError::UnknownRoute));
        assert!(call(&store, &login, "WhoAmI", NOW).await.is_ok(), "any session may ask who it is");
        assert!(call(&store, &login, "Logout", NOW).await.is_ok());
    }

    #[tokio::test]
    async fn logout_ends_the_session_and_whoami_reports_the_token() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["node:read", "channels:read"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        let me = store.whoami(&login.session_id).unwrap();
        assert_eq!((me.token_id.as_str(), me.token_name.as_str()), (token_id(1).as_str(), "operator-1"));
        assert_eq!(me.permissions, ["node:read", "channels:read"]);

        store.logout(&login.session_id);
        assert!(store.whoami(&login.session_id).is_none());
        assert_eq!(call(&store, &login, "GetNodeInfo", NOW).await.err(), Some(VerifyError::Unknown));
    }

    #[tokio::test]
    async fn a_session_ends_after_twelve_hours() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();

        assert!(call(&store, &login, "GetNodeInfo", NOW + TWELVE_HOURS - 1).await.is_ok());
        assert_eq!(call(&store, &login, "GetNodeInfo", NOW + TWELVE_HOURS).await.err(), Some(VerifyError::Unknown));
        assert_eq!(fake.checks(), 2, "an expired session does not ask LDK Server");
        assert_eq!(session_count(&store), 0);
    }

    #[tokio::test]
    async fn live_ids_are_reread_at_most_once_a_minute_and_a_failed_read_keeps_the_old_list() {
        let fake = FakeAuthority::new();
        let known = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        store.refresh_live_ids(NOW).await;
        assert_eq!(fake.live_id_reads(), 1);

        let made_elsewhere = fake.add(2, &["admin"]);
        assert_eq!(store.login(&made_elsewhere, NOW + 30).await.err(), Some(LoginError::UnknownId));
        assert_eq!(fake.live_id_reads(), 1, "read less than a minute ago");
        assert!(store.login(&made_elsewhere, NOW + 60).await.is_ok());
        assert_eq!(fake.live_id_reads(), 2);

        *fake.live.lock().unwrap() = None;
        store.refresh_live_ids(NOW + 120).await;
        assert_eq!(fake.live_id_reads(), 3);
        assert!(store.login(&known, NOW + 121).await.is_ok(), "a failed read keeps the old list");
    }

    #[tokio::test]
    async fn a_token_the_daemon_lists_or_unlists_counts_at_once_without_a_list_read() {
        let fake = FakeAuthority::new();
        let first = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        store.login(&first, NOW).await.unwrap();
        let made = fake.add(2, &["node:read"]);

        store.add_live_id(&token_id(2));
        assert!(store.login(&made, NOW + 1).await.is_ok(), "made by the daemon, usable before the next read");
        store.remove_live_id(&token_id(1));
        assert_eq!(store.login(&first, NOW + 2).await.err(), Some(LoginError::UnknownId));
        assert_eq!(fake.live_id_reads(), 1);
        assert_eq!(fake.checks(), 2, "the unlisted token is refused without asking LDK Server");
    }

    #[tokio::test(start_paused = true)]
    async fn a_login_that_races_a_revoke_makes_no_session() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        store.refresh_live_ids(NOW).await;
        *fake.check_delay.lock().unwrap() = Duration::from_millis(50);

        let racing = {
            let (store, token) = (Arc::clone(&store), token.clone());
            tokio::spawn(async move { store.login(&token, NOW).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        store.remove_live_id(&token_id(1));
        store.drop_token(&token_id(1));

        assert_eq!(racing.await.unwrap().err(), Some(LoginError::UnknownId));
        assert_eq!(fake.checks(), 1, "LDK Server accepted it before the revoke landed");
        assert_eq!(session_count(&store), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_list_read_that_overlaps_an_add_or_remove_does_not_undo_it() {
        let fake = FakeAuthority::new();
        fake.add(1, &["admin"]);
        let store = Arc::new(store(&fake, None));
        store.refresh_live_ids(NOW).await;
        *fake.list_delay.lock().unwrap() = Duration::from_millis(50);

        let slow_read = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.refresh_live_ids(NOW + 60).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        store.add_live_id(&token_id(2));
        store.remove_live_id(&token_id(1));
        slow_read.await.unwrap();

        assert!(store.lock().live_ids.contains(&token_id(2)), "the add outlives a read that began before it");
        assert!(!store.lock().live_ids.contains(&token_id(1)), "the remove outlives a read that began before it");
        assert_eq!(fake.live_id_reads(), 2);

        *fake.list_delay.lock().unwrap() = Duration::ZERO;
        fake.live.lock().unwrap().as_mut().unwrap().remove(&token_id(1));
        fake.add(3, &["admin"]);
        store.refresh_live_ids(NOW + 120).await;
        let ids = store.lock().live_ids.clone();
        assert_eq!(ids, HashSet::from([token_id(3)]), "a later read is used as LDK Server gave it");
    }

    #[tokio::test]
    async fn a_check_that_ended_without_an_answer_does_not_leave_requests_spinning() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let store = store(&fake, None);
        let login = store.login(&token, NOW).await.unwrap();
        // As if the checking request had died after taking its answer channel but before answering.
        let (abandoned, in_flight) = watch::channel(None);
        store.lock().sessions.get_mut(&login.session_id).unwrap().checking = Some(in_flight);
        drop(abandoned);

        let request = call(&store, &login, "GetNodeInfo", NOW + 200);
        let answer = tokio::time::timeout(Duration::from_secs(2), request).await;

        assert!(answer.expect("the request is answered").is_ok());
        assert_eq!(fake.checks(), 2, "the login and a fresh check");
    }

    #[tokio::test]
    async fn an_unknown_id_while_ldk_cannot_list_tokens_is_an_outage_not_a_refusal() {
        let fake = FakeAuthority::new();
        let token = fake.add(1, &["admin"]);
        let listed = fake.live.lock().unwrap().take();
        let store = store(&fake, None);
        let unreachable = Some(LoginError::Unreachable("Can't reach LDK Server".into()));

        assert_eq!(store.login(&token, NOW).await.err(), unreachable);
        assert_eq!(store.login(&token, NOW + 30).await.err(), unreachable, "no second read within the minute");
        assert_eq!((fake.live_id_reads(), fake.checks()), (1, 0));

        *fake.live.lock().unwrap() = listed;
        assert!(store.login(&token, NOW + 60).await.is_ok());
        let stranger = Macaroon::mint(b"some other root key", token_id(99).as_bytes()).unwrap().to_hex();
        assert_eq!(store.login(&stranger, NOW + 61).await.err(), Some(LoginError::UnknownId));

        *fake.live.lock().unwrap() = None;
        *fake.list_error.lock().unwrap() = LdkServerErrorCode::AuthorizationError;
        assert_eq!(store.login(&stranger, NOW + 120).await.err(), Some(LoginError::UnknownId), "LDK Server answered");
    }

    #[tokio::test(start_paused = true)]
    async fn seeding_retries_until_ldk_lists_tokens_and_leaves_the_refresh_budget() {
        let fake = FakeAuthority::new();
        let known = fake.add(1, &["admin"]);
        let listed = fake.live.lock().unwrap().take();
        let store = Arc::new(store(&fake, None));

        let seeding = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.seed_live_ids().await })
        };
        tokio::time::sleep(Duration::from_secs(2)).await;
        *fake.live.lock().unwrap() = listed;
        tokio::time::timeout(Duration::from_secs(10), seeding).await.expect("seeding ends once LDK answers").unwrap();
        assert_eq!(fake.live_id_reads(), 3, "tried at 0 s and 1 s, then read at 3 s");

        assert!(store.login(&known, NOW).await.is_ok());
        assert_eq!(fake.live_id_reads(), 3, "the seeded list knew it");
        let made_elsewhere = fake.add(2, &["admin"]);
        assert!(store.login(&made_elsewhere, NOW).await.is_ok(), "seeding left the once-a-minute read unspent");
        assert_eq!(fake.live_id_reads(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn seeding_stops_when_ldk_refuses_to_list_tokens() {
        let fake = FakeAuthority::new();
        *fake.live.lock().unwrap() = None;
        *fake.list_error.lock().unwrap() = LdkServerErrorCode::AuthorizationError;
        let store = store(&fake, None);

        tokio::time::timeout(Duration::from_secs(3600), store.seed_live_ids()).await.expect("no retry after a refusal");

        assert_eq!(fake.live_id_reads(), 1);
    }

    #[tokio::test]
    async fn login_and_verify_can_run_on_any_thread() {
        fn send<T: Send>(_: &T) {}
        let fake = FakeAuthority::new();
        let store = store(&fake, None);
        send(&store.login("", NOW));
        send(&store.verify(None, "GetNodeInfo", b"", NOW));
        send(&store.refresh_live_ids(NOW));
        send(&store.seed_live_ids());
    }
}
