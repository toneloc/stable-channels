//! GUI logins: what the daemon asks LDK Server about tokens, and the sessions it keeps.

pub mod gui_token;
pub mod handlers;
pub mod manage;
pub mod middleware;
pub mod sessions;
#[cfg(test)]
pub mod testing;
pub mod tokens;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sc_protos::access::allows;
use tracing::{info, warn};

use crate::access::sessions::SessionStore;
use crate::access::tokens::TokenFacts;
use crate::state::AppState;

/// What GUI logins need once LDK Server has said who the daemon's own token is.
pub struct Access {
    pub sessions: SessionStore,
    /// The token the daemon itself calls LDK Server with.
    pub own_token_id: String,
    /// Whether that token is an unrestricted admin, the only kind that can make the GUI admin token.
    pub own_is_admin: bool,
}

/// Opens GUI logins, reads LDK Server's token list for the first logins, then makes sure the GUI admin token works.
pub async fn start(state: &AppState, own: TokenFacts) {
    let lists = lists_tokens(&own);
    if !lists {
        warn!("GUI logins need the daemon's LDK token to have macaroons:manage");
    }
    open(state, own);
    let Some(access) = state.access.get() else {
        return;
    };
    // Without macaroons:manage LDK Server refuses the list, so there is nothing to wait for.
    if lists {
        access.sessions.seed_live_ids().await;
    }
    gui_token::ensure(state, None).await;
}

/// Opens GUI logins, with sessions capped at the daemon token's permissions unless it is an unrestricted admin.
pub fn open(state: &AppState, own: TokenFacts) {
    let mut opened = false;
    let access = state.access.get_or_init(|| {
        opened = true;
        let cap = session_cap(&own);
        if let Some(cap) = &cap {
            warn!(
                "the daemon's LDK Server token {} is not an unrestricted admin; GUI sessions get at most its permissions: {}",
                own.id,
                cap.join(", ")
            );
        }
        let own_is_admin = cap.is_none();
        Access { sessions: SessionStore::new(Arc::clone(&state.tokens), cap), own_token_id: own.id, own_is_admin }
    });
    if opened {
        info!("GUI logins are open; the daemon calls LDK Server with token {}", access.own_token_id);
    }
}

/// None for an unrestricted admin token; otherwise its permissions, which no session may go beyond.
fn session_cap(own: &TokenFacts) -> Option<Vec<String>> {
    let unrestricted =
        own.permissions == ["admin"] && own.caveats.iter().all(|caveat| caveat == "permissions = admin");
    (!unrestricted).then(|| own.permissions.clone())
}

/// Whether LDK Server lists its tokens to the daemon's token, which every GUI login needs.
fn lists_tokens(own: &TokenFacts) -> bool {
    allows(&own.permissions, "macaroons:manage")
}

/// Seconds since the Unix epoch, as sessions count time.
pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use crate::access::testing::{offline_state_before_logins, FakeLdk};

    fn own(permissions: &[&str], caveats: &[&str]) -> TokenFacts {
        TokenFacts {
            id: "d43a0c1e5b7f92a6e8c4b0d2f6a81e37".into(),
            name: "admin".into(),
            permissions: permissions.iter().map(|p| p.to_string()).collect(),
            caveats: caveats.iter().map(|c| c.to_string()).collect(),
        }
    }

    fn strings(list: &[&str]) -> Option<Vec<String>> {
        Some(list.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn only_an_unrestricted_admin_token_leaves_sessions_uncapped() {
        assert_eq!(session_cap(&own(&["admin"], &["permissions = admin"])), None);
        assert_eq!(session_cap(&own(&["admin"], &[])), None);
        let expiring = own(&["admin"], &["permissions = admin", "time-before = 1800000000"]);
        assert_eq!(session_cap(&expiring), strings(&["admin"]));
        let readonly = own(&["node:read", "channels:read"], &["permissions = node:read,channels:read"]);
        assert_eq!(session_cap(&readonly), strings(&["node:read", "channels:read"]));
        let narrowed = own(&["node:read"], &["permissions = admin", "permissions = node:read"]);
        assert_eq!(session_cap(&narrowed), strings(&["node:read"]));
    }

    #[test]
    fn gui_logins_need_a_daemon_token_that_can_list_tokens() {
        assert!(lists_tokens(&own(&["admin"], &["permissions = admin"])));
        assert!(lists_tokens(&own(&["node:read", "macaroons:manage"], &["permissions = node:read,macaroons:manage"])));
        assert!(!lists_tokens(&own(&["node:read", "channels:read"], &["permissions = node:read,channels:read"])));
    }

    #[tokio::test(start_paused = true)]
    async fn startup_reads_the_token_list_until_ldk_answers_then_makes_the_gui_token() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let operator = fake.add(1, "operator", &["admin"]);
        fake.list_outages.store(2, Ordering::SeqCst);
        let state = offline_state_before_logins(dir.path(), &fake);

        let started = tokio::time::timeout(Duration::from_secs(3600), start(&state, fake.daemon())).await;

        started.expect("startup ends once LDK Server lists its tokens");
        assert_eq!(fake.live_id_reads.load(Ordering::SeqCst), 3, "two outages, then the list");
        let access = state.access.get().unwrap();
        assert!(access.sessions.login(&operator, unix_now()).await.is_ok());
        let gui = std::fs::read_to_string(gui_token::path(&state)).expect("the GUI token is made after the list");
        assert!(access.sessions.login(gui.trim(), unix_now()).await.is_ok());
        assert_eq!(fake.live_id_reads.load(Ordering::SeqCst), 3, "both logins used the startup list");
    }

    #[tokio::test(start_paused = true)]
    async fn startup_without_macaroons_manage_reads_no_list_and_makes_no_gui_token() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        fake.set_daemon(&["node:read", "channels:read"]);
        let state = offline_state_before_logins(dir.path(), &fake);

        let started = tokio::time::timeout(Duration::from_secs(3600), start(&state, fake.daemon())).await;

        started.expect("startup does not wait on a list LDK Server will refuse");
        assert_eq!(fake.live_id_reads.load(Ordering::SeqCst), 0);
        assert!(!gui_token::path(&state).exists());
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
        assert!(state.access.get().is_some(), "logins are open, capped at the daemon token");
    }
}
