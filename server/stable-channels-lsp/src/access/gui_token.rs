//! The GUI admin token: an LDK Server admin token kept at `<data_dir>/<network>/gui.macaroon` for the first login.

use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::access::tokens::{token_root_id, TokenCheck};
use crate::access::{unix_now, Access};
use crate::secret_file;
use crate::state::AppState;

/// Where the GUI admin token is kept, as hex and a newline.
pub fn path(state: &AppState) -> PathBuf {
    state.data_dir.join(&state.network).join("gui.macaroon")
}

/// Keeps gui.macaroon unless it holds `revoked` or LDK Server refuses it, else makes a new admin token; returns its id.
pub async fn ensure(state: &AppState, revoked: Option<&str>) -> Option<String> {
    let access = state.access.get()?;
    // Held throughout, so startup and a revoke can't both make a new token.
    let mut gui_token_id = state.gui_token_id.lock().await;
    let path = path(state);
    *gui_token_id = match kept(state, &path, revoked).await {
        Some(id) => Some(id),
        None if !access.own_is_admin => {
            warn!(
                "the daemon's LDK Server token is not an unrestricted admin, so it makes no GUI admin token at {}",
                path.display()
            );
            None
        }
        None => make(state, access, &path).await,
    };
    gui_token_id.clone()
}

/// The file's token id unless it is `revoked` or LDK Server refused it; no answer keeps it, as an outage is no refusal.
async fn kept(state: &AppState, path: &Path, revoked: Option<&str>) -> Option<String> {
    std::fs::symlink_metadata(path).ok()?;
    let token = match crate::read_macaroon_file(path) {
        Ok(token) => token,
        Err(error) => {
            warn!("the GUI admin token file can't be used and will be replaced: {error:#}");
            return None;
        }
    };
    let Some(id) = token_root_id(&token) else {
        warn!("{} holds no LDK Server token and will be replaced", path.display());
        return None;
    };
    // Revoked through the daemon a moment ago, so no answer from LDK Server can keep it.
    if revoked == Some(id.as_str()) {
        info!("GUI admin token {id} was revoked; making a new one");
        return None;
    }
    match state.tokens.check(&token).await {
        TokenCheck::Accepted(facts) => Some(facts.id),
        TokenCheck::Unreachable(why) => {
            warn!("could not ask LDK Server about GUI admin token {id}, so it is kept: {why}");
            Some(id)
        }
        TokenCheck::Refused(why) => {
            info!("LDK Server refused GUI admin token {id} ({why}); making a new one");
            None
        }
    }
}

/// Makes an admin token with the daemon's own token and saves it; the token itself is never logged.
async fn make(state: &AppState, access: &Access, path: &Path) -> Option<String> {
    // LDK Server needs unique names, so each GUI token is named for when it was made.
    let name = format!("sc-gui-admin-{}", unix_now());
    let (facts, token) = match state.tokens.create_as_daemon(&name, vec!["admin".to_string()]).await {
        Ok(made) => made,
        Err(error) => {
            warn!("could not make the GUI admin token: {}", error.message);
            return None;
        }
    };
    // Listed before the file appears, so a GUI that reads the file at once can log in.
    access.sessions.add_live_id(&facts.id);
    let saved = path
        .parent()
        .map_or(Ok(()), secret_file::create_private_dir_all)
        .and_then(|()| secret_file::replace(path, format!("{token}\n").as_bytes(), 0o600));
    if let Err(error) = saved {
        warn!(
            "made GUI admin token {} but could not save it to {}: {error}; it can be revoked on the Access page",
            facts.id,
            path.display()
        );
        return None;
    }
    info!(
        "saved new GUI admin token {} to {}; an older GUI token may still exist in LDK Server \
         and can be revoked on the Access page",
        facts.id,
        path.display()
    );
    Some(facts.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::Ordering;

    use crate::access::testing::{offline_state, offline_state_before_logins, open_as, test_macaroon, Call, FakeLdk};
    use crate::access::tokens::token_root_id;
    use crate::access::unix_now;

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn startup_replaces_a_refused_gui_token_and_keeps_a_good_one() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let state = offline_state(dir.path(), &fake);
        let path = path(&state);

        let made = ensure(&state, None).await.expect("a missing GUI token is made");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, format!("{}\n", text.trim()), "hex and a newline");
        assert_eq!(token_root_id(text.trim()), Some(made.clone()));
        assert_eq!(mode(&path), 0o600);
        let Some(Call::CreateAsDaemon { name, permissions }) = fake.calls().pop() else { panic!("{:?}", fake.calls()) };
        assert_eq!(permissions, ["admin"]);
        let secs: u64 = name.strip_prefix("sc-gui-admin-").unwrap().parse().unwrap();
        assert!(secs.abs_diff(unix_now()) <= 5, "{name}");
        assert_eq!(state.gui_token_id.lock().await.as_deref(), Some(made.as_str()));

        assert_eq!(ensure(&state, None).await, Some(made.clone()), "a token LDK Server accepts is kept");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        fake.unreachable.store(true, Ordering::SeqCst);
        assert_eq!(ensure(&state, None).await, Some(made.clone()), "no answer is not a refusal");
        fake.unreachable.store(false, Ordering::SeqCst);
        assert_eq!(fake.calls().len(), 1, "no new token");

        fake.forget(&made);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let replaced = ensure(&state, None).await.expect("a refused GUI token is replaced");
        let new_text = std::fs::read_to_string(&path).unwrap();
        assert_ne!(replaced, made);
        assert_eq!(token_root_id(new_text.trim()), Some(replaced.clone()));
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fake.calls().len(), 2);
        assert_eq!(state.gui_token_id.lock().await.as_deref(), Some(replaced.as_str()));
        let access = state.access.get().unwrap();
        assert!(access.sessions.login(new_text.trim(), unix_now()).await.is_ok(), "it logs in at once");
    }

    #[tokio::test]
    async fn without_an_admin_daemon_token_no_gui_token_is_made() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let state = offline_state_before_logins(dir.path(), &fake);
        open_as(&state, &fake, &["node:read", "channels:read", "macaroons:manage"]);

        assert_eq!(ensure(&state, None).await, None);

        assert!(!path(&state).exists());
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
        assert_eq!(*state.gui_token_id.lock().await, None);
    }

    #[tokio::test]
    async fn a_gui_token_file_ldk_never_made_or_that_is_no_token_is_replaced() {
        let never_made = format!("{}\n", test_macaroon(b"0f1e2d3c4b5a69788796a5b4c3d2e1f0"));
        let strangers = [never_made.into_bytes(), vec![0x42u8; 32]];
        for stranger in strangers {
            let dir = tempfile::tempdir().unwrap();
            let fake = FakeLdk::new();
            let state = offline_state(dir.path(), &fake);
            let path = path(&state);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &stranger).unwrap();

            let made = ensure(&state, None).await.expect("the file is replaced");

            assert_eq!(token_root_id(std::fs::read_to_string(&path).unwrap().trim()), Some(made));
        }
    }
}
