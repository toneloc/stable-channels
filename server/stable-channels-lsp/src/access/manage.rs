//! ListTokens, CreateToken and RevokeToken: the GUI manages LDK Server tokens with the caller's own token.

use std::collections::BTreeSet;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::Extension;
use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
use ldk_server_client::ldk_server_grpc::error::ErrorCode;
use ldk_server_client::ldk_server_grpc::permissions::ALL_PERMISSIONS;
use ldk_server_client::macaroon::derive_macaroon;
use sc_protos::access::{
    CreateTokenRequest, CreateTokenResponse, ListTokensRequest, ListTokensResponse, RevokeTokenRequest,
    RevokeTokenResponse, TokenInfo,
};
use tracing::{info, warn};

use crate::access::middleware::CANT_REACH_LDK;
use crate::access::sessions::Caller;
use crate::access::tokens::TokenFacts;
use crate::access::{gui_token, unix_now, Access};
use crate::handlers::{decode_body, error_response, ok_response};
use crate::state::AppState;

/// Every live token, tagged as the daemon's own, the GUI admin token and the caller's.
pub async fn list_tokens(State(state): State<AppState>, Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    if let Err(response) = decode_body::<ListTokensRequest>(&body) {
        return response;
    }
    let Some(access) = state.access.get() else {
        return error_response(ErrorCode::InternalServerError, CANT_REACH_LDK);
    };
    let tokens = match state.tokens.list(&caller.token).await {
        Ok(tokens) => tokens,
        Err(error) => return ldk_failure(error),
    };
    let gui = state.gui_token_id.lock().await.clone();
    let tokens = tokens.into_iter().map(|facts| tagged(facts, access, gui.as_deref(), &caller)).collect();
    ok_response(ListTokensResponse { tokens })
}

/// Makes a token with the caller's token, or with a copy of it that expires, so LDK Server applies its own limits.
pub async fn create_token(State(state): State<AppState>, Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let request: CreateTokenRequest = match decode_body(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let name = request.name.trim();
    if let Err(why) = check_new_token(name, &request.permissions, request.expires_at, unix_now()) {
        return error_response(ErrorCode::InvalidRequestError, why);
    }
    let Some(access) = state.access.get() else {
        return error_response(ErrorCode::InternalServerError, CANT_REACH_LDK);
    };
    // LDK Server gives the new token every restriction of the token that makes it, the expiry included.
    let maker = match request.expires_at {
        0 => caller.token.clone(),
        expires_at => match derive_macaroon(&caller.token, &[format!("time-before = {expires_at}")]) {
            Ok(copy) => copy,
            Err(why) => return error_response(ErrorCode::InvalidRequestError, format!("can't add an expiry: {why}")),
        },
    };
    let (facts, token_hex) = match state.tokens.create(&maker, name, request.permissions).await {
        Ok(made) => made,
        Err(error) => return ldk_failure(error),
    };
    access.sessions.add_live_id(&facts.id);
    let permissions = facts.permissions.join(",");
    info!(by = %caller.token_id, "GUI made LDK Server token {} ({}) with {permissions}", facts.id, facts.name);
    let token = Some(tagged(facts, access, None, &caller));
    ok_response(CreateTokenResponse { token, token_hex })
}

/// Revokes a token with the caller's token and ends its sessions at once; the daemon's own token is never revoked here.
pub async fn revoke_token(State(state): State<AppState>, Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let request: RevokeTokenRequest = match decode_body(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    // LDK Server takes either case and stores lowercase, so every comparison here uses lowercase too.
    let id = request.id.to_ascii_lowercase();
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return error_response(ErrorCode::InvalidRequestError, "a token id is 32 hexadecimal characters");
    }
    let Some(access) = state.access.get() else {
        return error_response(ErrorCode::InternalServerError, CANT_REACH_LDK);
    };
    if id == access.own_token_id {
        return error_response(ErrorCode::InvalidRequestError, "revoking this would stop stability payments");
    }
    // Its own task, so a request that goes away can't stop it between LDK Server's revoke and the follow-up.
    match tokio::spawn(revoke_and_follow_up(state.clone(), caller, id)).await {
        Ok(Ok(())) => ok_response(RevokeTokenResponse {}),
        Ok(Err(error)) => ldk_failure(error),
        Err(error) => {
            warn!("token revoke stopped before it finished: {error}");
            error_response(ErrorCode::InternalServerError, "the revoke did not finish; check the token list")
        }
    }
}

/// Revokes with the caller's token, ends that token's sessions, and replaces the GUI admin token if it was the one.
async fn revoke_and_follow_up(state: AppState, caller: Caller, id: String) -> Result<(), LdkServerError> {
    state.tokens.revoke(&caller.token, &id).await?;
    if let Some(access) = state.access.get() {
        // Unlisted first, so a login that raced the revoke can't add a session after the drop.
        access.sessions.remove_live_id(&id);
        access.sessions.drop_token(&id);
    }
    info!(by = %caller.token_id, "GUI revoked LDK Server token {id}");
    let was_gui_token = state.gui_token_id.lock().await.as_deref() == Some(id.as_str());
    if was_gui_token {
        gui_token::ensure(&state, Some(&id)).await;
    }
    Ok(())
}

fn tagged(facts: TokenFacts, access: &Access, gui: Option<&str>, caller: &Caller) -> TokenInfo {
    TokenInfo {
        used_by_daemon: facts.id == access.own_token_id,
        is_gui_token: gui == Some(facts.id.as_str()),
        is_you: facts.id == caller.token_id,
        id: facts.id,
        name: facts.name,
        permissions: facts.permissions,
        caveats: facts.caveats,
    }
}

/// Why LDK Server would refuse this new token, checked first so bad input never reaches it.
fn check_new_token(name: &str, permissions: &[String], expires_at: u64, now: u64) -> Result<(), String> {
    // LDK Server's own name rule (validate_name in ldk-server's macaroons/policy.rs).
    let name_ok = (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !name_ok {
        return Err("a token name is 1 to 64 letters, numbers, hyphens or underscores".into());
    }
    if permissions.is_empty() {
        return Err("a token needs at least one permission".into());
    }
    if let Some(unknown) = permissions.iter().find(|p| !ALL_PERMISSIONS.contains(&p.as_str())) {
        return Err(format!("unknown permission: {unknown}"));
    }
    let distinct: BTreeSet<&str> = permissions.iter().map(String::as_str).collect();
    if distinct.contains("admin") && distinct.len() > 1 {
        return Err("admin can't be combined with other permissions".into());
    }
    if expires_at != 0 && expires_at <= now {
        return Err("the expiry must be in the future".into());
    }
    Ok(())
}

/// LDK Server's refusal in its own words, which never hold a token; anything else is "Can't reach LDK Server".
fn ldk_failure(error: LdkServerError) -> Response {
    match error.error_code {
        LdkServerErrorCode::AuthError | LdkServerErrorCode::AuthorizationError => {
            error_response(ErrorCode::AuthError, error.message)
        }
        LdkServerErrorCode::InvalidRequestError => error_response(ErrorCode::InvalidRequestError, error.message),
        _ => {
            warn!("token management: no answer from LDK Server: {}", error.message);
            error_response(ErrorCode::InternalServerError, CANT_REACH_LDK)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use axum::http::StatusCode;
    use ldk_server_client::ldk_server_grpc::error::ErrorResponse;
    use ldk_server_client::ldk_server_grpc::permissions::READONLY_PERMISSIONS;
    use ldk_server_client::macaroon::Macaroon;
    use prost::Message;
    use sc_protos::access::{LoginRequest, SESSION_UNKNOWN};

    use crate::access::testing::{call, login, offline_state, post, refusal, signed_post, Call, FakeLdk, DAEMON_TOKEN_ID};
    use crate::access::tokens::token_root_id;
    use crate::build_router;

    fn create(name: &str, permissions: &[&str], expires_at: u64) -> Vec<u8> {
        let permissions = permissions.iter().map(|p| p.to_string()).collect();
        CreateTokenRequest { name: name.into(), permissions, expires_at }.encode_to_vec()
    }

    fn revoke(id: &str) -> Vec<u8> {
        RevokeTokenRequest { id: id.into() }.encode_to_vec()
    }

    fn id_of(token: &str) -> String {
        token_root_id(token).unwrap()
    }

    #[tokio::test]
    async fn the_list_tags_the_daemon_token_the_gui_token_and_you() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let me = fake.add(1, "operator", &["admin"]);
        let auditor = fake.add(2, "auditor", &READONLY_PERMISSIONS);
        let state = offline_state(dir.path(), &fake);
        let gui = gui_token::ensure(&state, None).await.expect("an admin daemon token makes the GUI token");
        let router = build_router(state);

        let (status, bytes) = signed_post(&router, &me, "ListTokens", ListTokensRequest {}.encode_to_vec()).await;

        assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
        let rows = ListTokensResponse::decode(&bytes[..]).unwrap().tokens;
        let tags: HashMap<String, (bool, bool, bool)> =
            rows.iter().map(|t| (t.id.clone(), (t.used_by_daemon, t.is_gui_token, t.is_you))).collect();
        let expected = HashMap::from([
            (DAEMON_TOKEN_ID.to_string(), (true, false, false)),
            (gui, (false, true, false)),
            (id_of(&me), (false, false, true)),
            (id_of(&auditor), (false, false, false)),
        ]);
        assert_eq!(tags, expected, "(used by daemon, GUI token, you)");
        let row = rows.iter().find(|t| t.name == "auditor").unwrap();
        assert_eq!(row.permissions, READONLY_PERMISSIONS);
        assert_eq!(row.caveats, [format!("permissions = {}", READONLY_PERMISSIONS.join(","))]);
        assert_eq!(fake.calls().last(), Some(&Call::List { token: me }), "listed with the caller's own token");
    }

    #[tokio::test]
    async fn a_created_token_with_an_expiry_is_made_from_an_expiring_copy() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let me = fake.add(1, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let session = login(&router, &me).await;
        let expires_at = unix_now() + 3600;

        let body = create("  night-shift ", &["node:read", "payments:read"], expires_at);
        let (status, bytes) = call(&router, &session, "CreateToken", body).await;

        assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
        let made = CreateTokenResponse::decode(&bytes[..]).unwrap();
        let info = made.token.clone().unwrap();
        assert_eq!(info.name, "night-shift");
        assert_eq!(info.permissions, ["node:read", "payments:read"]);
        assert!(info.caveats.contains(&format!("time-before = {expires_at}")), "{:?}", info.caveats);
        assert_eq!(token_root_id(&made.token_hex), Some(info.id.clone()), "the secret is the new token");
        let Some(Call::Create { token, name, .. }) = fake.calls().pop() else { panic!("no create: {:?}", fake.calls()) };
        assert_eq!(name, "night-shift");
        assert_ne!(token, me, "not the caller's token as it is");
        assert_eq!(token_root_id(&token), token_root_id(&me), "a copy of the caller's token");
        let caveats: Vec<String> = Macaroon::from_hex(&token)
            .unwrap()
            .caveats()
            .iter()
            .map(|c| String::from_utf8(c.clone()).unwrap())
            .collect();
        assert_eq!(caveats, [format!("time-before = {expires_at}")]);
        assert_eq!(login(&router, &made.token_hex).await.token_id, info.id, "the new token logs in at once");

        let (status, _) = call(&router, &session, "CreateToken", create("day-shift", &["node:read"], 0)).await;
        assert_eq!(status, StatusCode::OK);
        let without_expiry = Call::Create { token: me, name: "day-shift".into(), permissions: vec!["node:read".into()] };
        assert_eq!(fake.calls().last(), Some(&without_expiry), "made with the caller's own token");
    }

    #[tokio::test]
    async fn a_past_expiry_or_blank_name_is_refused_without_asking_ldk() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let me = fake.add(1, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let session = login(&router, &me).await;
        let now = unix_now();
        let name_rule = "a token name is 1 to 64 letters, numbers, hyphens or underscores";
        let long = "n".repeat(65);
        let cases: Vec<(&str, &[&str], u64, &str)> = vec![
            ("", &["node:read"], 0, name_rule),
            ("   ", &["node:read"], 0, name_rule),
            ("night shift", &["node:read"], 0, name_rule),
            (&long, &["node:read"], 0, name_rule),
            ("night-shift", &["node:read"], now - 1, "the expiry must be in the future"),
            ("night-shift", &["node:read"], 1, "the expiry must be in the future"),
            ("night-shift", &[], 0, "a token needs at least one permission"),
            ("night-shift", &["node:write"], 0, "unknown permission: node:write"),
            ("night-shift", &["admin", "node:read"], 0, "admin can't be combined with other permissions"),
        ];

        for (name, permissions, expires_at, message) in cases {
            let (status, bytes) = call(&router, &session, "CreateToken", create(name, permissions, expires_at)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name:?} {permissions:?} {expires_at}");
            assert_eq!(refusal(&bytes), (ErrorCode::InvalidRequestError, message.to_string()), "{name:?}");
        }
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    }

    #[tokio::test]
    async fn revoking_the_daemon_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let me = fake.add(1, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let session = login(&router, &me).await;

        for id in [DAEMON_TOKEN_ID.to_string(), DAEMON_TOKEN_ID.to_uppercase()] {
            let (status, bytes) = call(&router, &session, "RevokeToken", revoke(&id)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{id}");
            let expected = (ErrorCode::InvalidRequestError, "revoking this would stop stability payments".to_string());
            assert_eq!(refusal(&bytes), expected, "{id}");
        }
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
        assert!(fake.holds(DAEMON_TOKEN_ID));
    }

    #[tokio::test]
    async fn revoking_a_token_ends_its_sessions_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let admin = fake.add(1, "operator", &["admin"]);
        let doomed = fake.add(2, "laptop", &READONLY_PERMISSIONS);
        let kept = fake.add(3, "phone", &READONLY_PERMISSIONS);
        let router = build_router(offline_state(dir.path(), &fake));
        let operator = login(&router, &admin).await;
        let gone = [login(&router, &doomed).await, login(&router, &doomed).await];
        let other = login(&router, &kept).await;

        let (status, bytes) = call(&router, &operator, "RevokeToken", revoke(&id_of(&doomed).to_uppercase())).await;

        assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
        assert_eq!(fake.calls().last(), Some(&Call::Revoke { token: admin, id: id_of(&doomed) }));
        for session in &gone {
            let (status, bytes) = call(&router, session, "WhoAmI", vec![]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(refusal(&bytes), (ErrorCode::AuthError, SESSION_UNKNOWN.to_string()), "no 30 s wait");
        }
        assert_eq!(call(&router, &other, "WhoAmI", vec![]).await.0, StatusCode::OK, "other tokens keep their sessions");
        let (status, _) = post(&router, "/Login", None, LoginRequest { token: doomed }.encode_to_vec()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoking_the_gui_token_writes_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let admin = fake.add(1, "operator", &["admin"]);
        let state = offline_state(dir.path(), &fake);
        let first = gui_token::ensure(&state, None).await.expect("an admin daemon token makes the GUI token");
        let path = gui_token::path(&state);
        let before = std::fs::read_to_string(&path).unwrap();
        let router = build_router(state.clone());
        let session = login(&router, &admin).await;

        let (status, bytes) = call(&router, &session, "RevokeToken", revoke(&first)).await;

        assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
        let after = std::fs::read_to_string(&path).unwrap();
        assert_ne!(after, before);
        assert_eq!(after, format!("{}\n", after.trim()), "hex and a newline");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let second = id_of(after.trim());
        assert_ne!(second, first);
        assert!(!fake.holds(&first) && fake.holds(&second));
        assert_eq!(state.gui_token_id.lock().await.as_deref(), Some(second.as_str()));
        assert_eq!(login(&router, after.trim()).await.token_id, second, "the new GUI token logs in at once");
    }

    #[tokio::test]
    async fn revoking_the_gui_token_replaces_it_even_when_ldk_gives_no_answer_about_it() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let admin = fake.add(1, "operator", &["admin"]);
        let state = offline_state(dir.path(), &fake);
        let first = gui_token::ensure(&state, None).await.expect("an admin daemon token makes the GUI token");
        let path = gui_token::path(&state);
        let router = build_router(state.clone());
        let session = login(&router, &admin).await;
        fake.unreachable.store(true, Ordering::SeqCst);

        let (status, bytes) = call(&router, &session, "RevokeToken", revoke(&first)).await;

        assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
        let second = id_of(std::fs::read_to_string(&path).unwrap().trim());
        assert_ne!(second, first, "the revoked token is not kept for want of an answer");
        assert!(fake.holds(&second));
        assert_eq!(state.gui_token_id.lock().await.as_deref(), Some(second.as_str()));
    }

    #[tokio::test]
    async fn the_gui_token_is_replaced_even_if_the_revoke_request_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let admin = fake.add(1, "operator", &["admin"]);
        let state = offline_state(dir.path(), &fake);
        let first = gui_token::ensure(&state, None).await.expect("an admin daemon token makes the GUI token");
        let path = gui_token::path(&state);
        let router = build_router(state.clone());
        let operator = login(&router, &admin).await;
        let gui_session = login(&router, std::fs::read_to_string(&path).unwrap().trim()).await;

        // LDK Server revokes, but its answer only comes after the request has gone.
        let answer = fake.revoke_answer.lock().await;
        let request = call(&router, &operator, "RevokeToken", revoke(&first));
        assert!(tokio::time::timeout(Duration::from_millis(200), request).await.is_err(), "still waiting for LDK");
        assert!(!fake.holds(&first));
        drop(answer);

        let mut second = first.clone();
        for _ in 0..250 {
            second = id_of(std::fs::read_to_string(&path).unwrap().trim());
            if second != first {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_ne!(second, first, "the GUI token is replaced after the request went away");
        assert!(fake.holds(&second));
        let (status, bytes) = call(&router, &gui_session, "WhoAmI", vec![]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refusal(&bytes), (ErrorCode::AuthError, SESSION_UNKNOWN.to_string()), "its sessions ended too");
    }

    #[tokio::test]
    async fn a_read_only_session_cannot_reach_token_management() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let reader = fake.add(1, "auditor", &READONLY_PERMISSIONS);
        let other = fake.add(2, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let session = login(&router, &reader).await;

        let requests = [
            ("ListTokens", ListTokensRequest {}.encode_to_vec()),
            ("CreateToken", create("sneaky", &["admin"], 0)),
            ("RevokeToken", revoke(&id_of(&other))),
        ];
        for (route, body) in requests {
            let (status, bytes) = call(&router, &session, route, body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{route}");
            let expected = (ErrorCode::AuthError, "this token lacks macaroons:manage".to_string());
            assert_eq!(refusal(&bytes), expected, "{route}");
        }
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
        assert!(fake.holds(&id_of(&other)));
    }

    #[tokio::test]
    async fn ldk_refusals_reach_the_gui_in_ldks_words() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let helper = fake.add(1, "helper", &["macaroons:manage", "node:read"]);
        fake.add(2, "taken", &["node:read"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let session = login(&router, &helper).await;

        let (status, bytes) = call(&router, &session, "CreateToken", create("bigger", &["admin"], 0)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let expected = (ErrorCode::AuthError, "Cannot grant a permission that the calling root does not have".to_string());
        assert_eq!(refusal(&bytes), expected);
        let (status, bytes) = call(&router, &session, "CreateToken", create("taken", &["node:read"], 0)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(refusal(&bytes), (ErrorCode::InvalidRequestError, "macaroon name already exists: taken".to_string()));
    }

    #[tokio::test]
    async fn each_ldk_failure_has_its_code_and_words() {
        let words = "macaroon name already exists: night-shift";
        let cases = [
            (LdkServerErrorCode::AuthError, StatusCode::UNAUTHORIZED, ErrorCode::AuthError, words),
            (LdkServerErrorCode::AuthorizationError, StatusCode::UNAUTHORIZED, ErrorCode::AuthError, words),
            (LdkServerErrorCode::InvalidRequestError, StatusCode::BAD_REQUEST, ErrorCode::InvalidRequestError, words),
        ];
        let outages =
            [LdkServerErrorCode::InternalError, LdkServerErrorCode::InternalServerError, LdkServerErrorCode::LightningError];
        let outages = outages.map(|ldk_code| {
            (ldk_code, StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalServerError, "Can't reach LDK Server")
        });
        for (ldk_code, status, code, message) in cases.into_iter().chain(outages) {
            let label = format!("{ldk_code:?}");
            let response = ldk_failure(LdkServerError::new(ldk_code, words));
            assert_eq!(response.status(), status, "{label}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body = ErrorResponse::decode(&bytes[..]).unwrap();
            assert_eq!((body.error_code, body.message.as_str()), (code as i32, message), "{label}");
        }
    }
}
