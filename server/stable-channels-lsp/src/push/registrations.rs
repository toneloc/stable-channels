//! Wallet push registrations. A token is stored only for a node that has a channel with the LSP, the only nodes a wake push ever goes to, so what an unauthenticated caller can store is bounded by the channels; a node with no channel yet waits in a small in-memory list until its channel opens.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::tokens::{self, SaveOutcome};

/// Most registrations held for nodes with no channel yet; past this the one waiting longest is dropped.
const MAX_HELD: usize = 1_000;

/// What became of a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// Stored for a node with a channel.
    Saved,
    /// Held in memory until the node's channel opens.
    Held,
    Failed,
}

#[derive(Clone)]
struct Registration {
    token: String,
    platform: String,
    environment: String,
    /// Unix seconds it arrived at; a held registration is stored only if nothing newer was stored for its node meanwhile.
    arrived_at: i64,
}

#[derive(Default)]
struct State {
    /// Node ids, in lowercase hex, that have a channel with the LSP, as last reported.
    channel_peers: HashSet<String>,
    /// The latest registration of each node with no channel yet, and the nodes from the one waiting longest.
    held: HashMap<String, Registration>,
    held_order: VecDeque<String>,
}

pub struct Registrations {
    data_dir: String,
    state: Mutex<State>,
    /// Token writes take turns, so a flood of registrations cannot pile SQLite writers onto the runtime. The stability tick never waits on it.
    writes: tokio::sync::Mutex<()>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Registrations {
    pub fn new(data_dir: &Path) -> Self {
        tokens::init_db(data_dir);
        Self {
            data_dir: data_dir.to_string_lossy().to_string(),
            state: Mutex::new(State::default()),
            writes: tokio::sync::Mutex::new(()),
        }
    }

    /// Store a registration for a node with a channel, or hold it until its channel opens. The node id is kept in lowercase, the form wake lookups use.
    pub async fn register(&self, token: &str, platform: &str, node_id: &str, environment: &str) -> RegisterOutcome {
        let node_id = node_id.to_ascii_lowercase();
        let registration = Registration {
            token: token.to_owned(),
            platform: platform.to_owned(),
            environment: environment.to_owned(),
            arrived_at: unix_now(),
        };
        {
            let mut state = self.state();
            if !state.channel_peers.contains(&node_id) {
                state.hold(node_id, registration);
                return RegisterOutcome::Held;
            }
            // This registration is newer than anything held for the node.
            state.release(&node_id);
        }
        let _write = self.writes.lock().await;
        match tokens::save_token(&self.data_dir, &registration.token, &registration.platform, &node_id, &registration.environment) {
            SaveOutcome::Saved | SaveOutcome::Superseded => RegisterOutcome::Saved,
            SaveOutcome::Failed => RegisterOutcome::Failed,
        }
    }

    /// Replace the set of nodes with a channel at once, and store what was held for any of them on a separate task, so the caller never waits on a write.
    pub fn set_channel_peers(self: &Arc<Self>, peers: HashSet<String>) -> tokio::task::JoinHandle<()> {
        let ready: Vec<(String, Registration)> = {
            let mut state = self.state();
            let ready: Vec<_> = peers.iter().filter_map(|node| state.release(node).map(|registration| (node.clone(), registration))).collect();
            state.channel_peers = peers;
            ready
        };
        let registrations = Arc::clone(self);
        tokio::spawn(async move {
            if ready.is_empty() {
                return;
            }
            let _write = registrations.writes.lock().await;
            for (node_id, registration) in ready {
                let saved = tokens::save_held_token(
                    &registrations.data_dir,
                    &registration.token,
                    &registration.platform,
                    &node_id,
                    &registration.environment,
                    registration.arrived_at,
                );
                if saved == SaveOutcome::Failed {
                    // Kept for the next pass rather than lost, unless the node has registered again since.
                    let mut state = registrations.state();
                    if !state.held.contains_key(&node_id) {
                        state.hold(node_id, registration);
                    }
                }
            }
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl State {
    /// Hold a node's latest registration; a node that registers again moves to the back.
    fn hold(&mut self, node_id: String, registration: Registration) {
        if self.held.insert(node_id.clone(), registration).is_some() {
            self.held_order.retain(|node| node != &node_id);
        }
        self.held_order.push_back(node_id);
        while self.held_order.len() > MAX_HELD {
            if let Some(oldest) = self.held_order.pop_front() {
                self.held.remove(&oldest);
            }
        }
    }

    /// Take a node's held registration out of the list.
    fn release(&mut self, node_id: &str) -> Option<Registration> {
        let registration = self.held.remove(node_id)?;
        self.held_order.retain(|node| node != node_id);
        Some(registration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const NODE: &str = "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b";

    fn registrations(dir: &Path) -> Arc<Registrations> {
        Arc::new(Registrations::new(dir))
    }

    fn stored_rows(data_dir: &Path) -> i64 {
        rusqlite::Connection::open(data_dir.join("push_tokens.db"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM push_tokens", [], |row| row.get(0))
            .unwrap()
    }

    fn token_of(data_dir: &Path, node_id: &str) -> Option<String> {
        tokens::load_token_for_node(&data_dir.to_string_lossy(), node_id).map(|info| info.token)
    }

    fn peers(node: &str) -> HashSet<String> {
        HashSet::from([node.to_owned()])
    }

    #[tokio::test]
    async fn a_node_without_a_channel_is_held_not_stored() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        assert_eq!(registrations.register("tok", "ios", NODE, "production").await, RegisterOutcome::Held);
        assert_eq!(stored_rows(dir.path()), 0, "nothing reaches the disk for a node no push can go to");
    }

    #[tokio::test]
    async fn a_held_registration_is_stored_when_its_channel_opens() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.register("first", "ios", NODE, "production").await;
        registrations.register("rotated", "ios", NODE, "production").await;
        registrations.set_channel_peers(peers(NODE)).await.unwrap();
        assert_eq!(token_of(dir.path(), NODE).as_deref(), Some("rotated"), "the latest registration is the one kept");
        assert_eq!(stored_rows(dir.path()), 1);
    }

    #[tokio::test]
    async fn a_node_with_a_channel_is_stored_at_once_and_held_again_once_it_has_none() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.set_channel_peers(peers(NODE)).await.unwrap();
        assert_eq!(registrations.register("tok", "android", NODE, "").await, RegisterOutcome::Saved);
        assert_eq!(token_of(dir.path(), NODE).as_deref(), Some("tok"));

        registrations.set_channel_peers(HashSet::new()).await.unwrap();
        assert_eq!(registrations.register("later", "android", NODE, "").await, RegisterOutcome::Held);
    }

    #[tokio::test]
    async fn the_peer_list_changes_without_waiting_for_a_write() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.register("tok", "ios", NODE, "").await;
        let busy = registrations.writes.lock().await;
        // A flood of registrations holding the write turn cannot hold up whoever reports the peers.
        let pending = registrations.set_channel_peers(peers(NODE));
        assert!(registrations.state().channel_peers.contains(NODE));
        drop(busy);
        pending.await.unwrap();
        assert_eq!(token_of(dir.path(), NODE).as_deref(), Some("tok"));
    }

    #[tokio::test]
    async fn a_held_token_never_replaces_a_newer_one() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.register("held", "ios", NODE, "").await;
        let pending = registrations.set_channel_peers(peers(NODE));
        // The node rotates its token before the held one is written.
        assert_eq!(registrations.register("rotated", "ios", NODE, "").await, RegisterOutcome::Saved);
        pending.await.unwrap();
        assert_eq!(token_of(dir.path(), NODE).as_deref(), Some("rotated"));
        assert_eq!(stored_rows(dir.path()), 1, "the older held token is not stored at all");
    }

    #[tokio::test]
    async fn a_held_registration_whose_save_fails_stays_held_but_never_beats_a_newer_one() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.register("old", "ios", NODE, "").await;
        let conn = rusqlite::Connection::open(dir.path().join("push_tokens.db")).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON push_tokens BEGIN SELECT RAISE(ABORT, 'forced'); END;").unwrap();
        registrations.set_channel_peers(peers(NODE)).await.unwrap();
        assert_eq!(stored_rows(dir.path()), 0);
        assert!(registrations.state().held.contains_key(NODE), "kept until a save succeeds");

        conn.execute_batch("DROP TRIGGER fail_save").unwrap();
        assert_eq!(registrations.register("new", "ios", NODE, "").await, RegisterOutcome::Saved);
        registrations.set_channel_peers(peers(NODE)).await.unwrap();
        assert_eq!(token_of(dir.path(), NODE).as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn node_ids_in_any_case_share_one_bucket() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.set_channel_peers(peers(NODE)).await.unwrap();
        let variants = [NODE.to_ascii_uppercase(), NODE.replacen('e', "E", 1), NODE.replacen('d', "D", 2), NODE.replacen('f', "F", 3), NODE.replacen('c', "C", 1), NODE.replacen('b', "B", 2), NODE.replacen('a', "A", 1)];
        for (i, variant) in variants.iter().enumerate() {
            assert_eq!(registrations.register(&format!("tok{i}"), "ios", variant, "").await, RegisterOutcome::Saved);
        }
        assert_eq!(stored_rows(dir.path()), 5, "a case variant is the same node, with the same 5 tokens at most");
        assert!(token_of(dir.path(), NODE).is_some(), "stored under the id wake lookups use");
    }

    #[tokio::test]
    async fn held_registrations_are_bounded_and_a_fresh_one_waits_the_least() {
        let dir = tempdir().unwrap();
        let registrations = registrations(dir.path());
        registrations.register("mine", "ios", NODE, "").await;
        for i in 0..(MAX_HELD - 1) {
            registrations.register(&format!("tok{i}"), "ios", &format!("02{i:064x}"), "").await;
        }
        // Registering again moves the node to the back, so newer junk is dropped before it.
        registrations.register("mine-again", "ios", NODE, "").await;
        for i in MAX_HELD..(MAX_HELD + 10) {
            registrations.register(&format!("tok{i}"), "ios", &format!("02{i:064x}"), "").await;
        }
        let state = registrations.state();
        assert_eq!((state.held.len(), state.held_order.len()), (MAX_HELD, MAX_HELD));
        assert_eq!(state.held.get(NODE).map(|registration| registration.token.as_str()), Some("mine-again"));
        assert!(!state.held.contains_key(&format!("02{:064x}", 0)), "the one waiting longest is dropped first");
    }
}
