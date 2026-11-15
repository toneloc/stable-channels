//! Builds the app's own force close of a channel from ldk-node's stored channel data, so the app
//! can broadcast it without the LSP. ldk-node does not broadcast the close of an anchor channel
//! with a peer in `trusted_peers_no_reserve`, and the apps list the LSP there.
//!
//! Nothing is broadcast here. Call it after the node force closed the channel, with the rows of
//! its store read in one query while the node is stopped.
//!
//! Why the close read from the store is never an old, revoked state: LDK revokes a commitment only
//! after the monitor update holding the next one is persisted. Until then the channel is frozen
//! ("preventing us from revoking old states", `ChannelMonitorUpdateStatus::InProgress`), even with
//! ldk-node's asynchronous persistence. So the latest commitment in the store may lag the node's
//! memory but is never revoked. After `force_close_channel` the channel gets no new commitments,
//! and one query reads each monitor together with its pending updates. Broadcasting an old state
//! would let the counterparty take the channel funds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use lightning::bitcoin::absolute::LockTime;
use lightning::bitcoin::bip32::Xpriv;
use lightning::bitcoin::consensus::encode::serialize_hex;
use lightning::bitcoin::secp256k1::Secp256k1;
use lightning::bitcoin::transaction::Version;
use lightning::bitcoin::{Amount, Network, ScriptBuf, Transaction, TxOut};
use lightning::chain::chaininterface::{
    BroadcasterInterface, ConfirmationTarget, FeeEstimator, TransactionType,
};
use lightning::events::bump_transaction::{AnchorDescriptor, BumpTransactionEvent};
use lightning::events::{Event, ReplayEvent};
use lightning::io;
use lightning::sign::ecdsa::EcdsaChannelSigner;
use lightning::sign::{KeysManager, SignerProvider};
use lightning::util::logger::{Logger, Record};
use lightning::util::persist::{KVStoreSync, MonitorUpdatingPersister};

uniffi::setup_scaffolding!();

/// One row of ldk-node's store: the `ldk_node_data` table of `ldk_node_data.sqlite`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct StoredValue {
    pub primary_namespace: String,
    pub secondary_namespace: String,
    pub key: String,
    pub value: Vec<u8>,
}

/// The signed force close of a channel. Nothing has been broadcast.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ForceCloseTransactions {
    pub close_txid: String,
    /// Our latest commitment transaction, signed by both sides.
    pub close_tx_hex: String,
    /// A child of the close that spends our anchor output entirely as fee (CPFP). Broadcast it
    /// after the close.
    pub anchor_child_txid: String,
    pub anchor_child_hex: String,
}

#[derive(Debug, PartialEq, uniffi::Error)]
pub enum ForceCloseError {
    /// The seed is not 64 bytes.
    InvalidSeed,
    ReadFailed {
        reason: String,
    },
    ChannelNotFound,
    /// No close came out of the channel data, for example for a channel without anchors.
    NoClose,
    SigningFailed,
}

// UniFFI needs it for the error.
impl std::fmt::Display for ForceCloseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Builds the signed force close of the channel `channel_id` (hex) and a fee bump for it. `seed` is
/// the 64-byte wallet seed: the BIP39 seed of the recovery words without a passphrase, or the bytes
/// of a `keys_seed` file.
#[uniffi::export]
pub fn build_force_close(
    channel_id: String,
    seed: Vec<u8>,
    stored_values: Vec<StoredValue>,
) -> Result<ForceCloseTransactions, ForceCloseError> {
    let seed: [u8; 64] = seed.try_into().map_err(|_| ForceCloseError::InvalidSeed)?;
    // ldk-node seeds LDK's KeysManager with the BIP32 master key of the wallet seed. The network
    // does not change that key.
    let xprv =
        Xpriv::new_master(Network::Bitcoin, &seed).map_err(|_| ForceCloseError::InvalidSeed)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let keys = Arc::new(KeysManager::new(
        &xprv.private_key.secret_bytes(),
        now.as_secs(),
        now.subsec_nanos(),
        true,
    ));

    let mut store: HashMap<(String, String), HashMap<String, Vec<u8>>> = HashMap::new();
    for value in stored_values {
        store
            .entry((value.primary_namespace, value.secondary_namespace))
            .or_default()
            .insert(value.key, value.value);
    }
    let store = Arc::new(StoredValues(store));
    let logger = Arc::new(NoopLogger);
    let broadcaster = Arc::new(NoBroadcast);
    let fees = Arc::new(FloorFee);
    // The reader ldk-node uses at start: each monitor plus its pending updates. The maximum of
    // pending updates only matters for writes.
    let persister = MonitorUpdatingPersister::new(
        Arc::clone(&store),
        Arc::clone(&logger),
        100,
        Arc::clone(&keys),
        Arc::clone(&keys),
        Arc::clone(&broadcaster),
        Arc::clone(&fees),
    );
    let (_, monitor) = persister
        .read_all_channel_monitors_with_updates()
        .map_err(|e| ForceCloseError::ReadFailed {
            reason: e.to_string(),
        })?
        .into_iter()
        .find(|(_, monitor)| {
            monitor
                .channel_id()
                .to_string()
                .eq_ignore_ascii_case(channel_id.trim())
        })
        .ok_or(ForceCloseError::ChannelNotFound)?;

    // For an anchor channel LDK hands the close over in this event. Once the node has seen a block
    // after the force close, the stored monitor already holds the close's claim, and LDK ignores
    // this one as a second claim. That event isn't stored; rebroadcasting the pending claims makes
    // it again.
    monitor.broadcast_latest_holder_commitment_txn(&*broadcaster, &*fees, &*logger);
    monitor.rebroadcast_pending_claims(&*broadcaster, &*fees, &*logger);
    let events = Mutex::new(Vec::new());
    let handler = |event: Event| -> Result<(), ReplayEvent> {
        events.lock().unwrap().push(event);
        Ok(())
    };
    let handler_ref = &handler;
    let _ = monitor.process_pending_events(&handler_ref, &*logger);
    for event in events.into_inner().unwrap() {
        if let Event::BumpTransaction(BumpTransactionEvent::ChannelClose {
            commitment_tx,
            anchor_descriptor,
            ..
        }) = event
        {
            let child = anchor_child(&keys, &anchor_descriptor)?;
            return Ok(ForceCloseTransactions {
                close_txid: commitment_tx.compute_txid().to_string(),
                close_tx_hex: serialize_hex(&commitment_tx),
                anchor_child_txid: child.compute_txid().to_string(),
                anchor_child_hex: serialize_hex(&child),
            });
        }
    }
    Err(ForceCloseError::NoClose)
}

/// The anchor transaction LDK builds when coin selection adds no input: our anchor output in, an
/// empty OP_RETURN out (65 bytes of non-witness data, the standard minimum).
fn anchor_child(
    keys: &KeysManager,
    anchor: &AnchorDescriptor,
) -> Result<Transaction, ForceCloseError> {
    let mut child = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![anchor.unsigned_tx_input()],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new_op_return([0, 0, 0]),
        }],
    };
    let params = &anchor.channel_derivation_parameters;
    let sig = keys
        .derive_channel_signer(params.keys_id)
        .sign_holder_keyed_anchor_input(
            &params.transaction_parameters,
            &child,
            0,
            &Secp256k1::new(),
        )
        .map_err(|()| ForceCloseError::SigningFailed)?;
    child.input[0].witness = anchor.tx_input_witness(&sig);
    Ok(child)
}

/// The rows of ldk-node's store, by namespace and key. Read only.
struct StoredValues(HashMap<(String, String), HashMap<String, Vec<u8>>>);

impl StoredValues {
    fn namespace(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
    ) -> Option<&HashMap<String, Vec<u8>>> {
        self.0.get(&(
            primary_namespace.to_string(),
            secondary_namespace.to_string(),
        ))
    }
}

impl KVStoreSync for StoredValues {
    fn read(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
        key: &str,
    ) -> Result<Vec<u8>, io::Error> {
        self.namespace(primary_namespace, secondary_namespace)
            .and_then(|values| values.get(key))
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not stored"))
    }

    fn write(&self, _: &str, _: &str, _: &str, _: Vec<u8>) -> Result<(), io::Error> {
        Err(io::Error::new(io::ErrorKind::Other, "read only"))
    }

    fn remove(&self, _: &str, _: &str, _: &str, _: bool) -> Result<(), io::Error> {
        Err(io::Error::new(io::ErrorKind::Other, "read only"))
    }

    fn list(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
    ) -> Result<Vec<String>, io::Error> {
        Ok(self
            .namespace(primary_namespace, secondary_namespace)
            .map(|values| values.keys().cloned().collect())
            .unwrap_or_default())
    }
}

struct NoopLogger;

impl Logger for NoopLogger {
    fn log(&self, _record: Record) {}
}

/// Drops what LDK would broadcast. The app broadcasts the close itself.
struct NoBroadcast;

impl BroadcasterInterface for NoBroadcast {
    fn broadcast_transactions(&self, _: &[(&Transaction, TransactionType)]) {}
}

/// Only sets the fee target of a later bump; the close itself is already signed with its fee.
struct FloorFee;

impl FeeEstimator for FloorFee {
    fn get_est_sat_per_1000_weight(&self, _: ConfirmationTarget) -> u32 {
        253
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(primary_namespace: &str, key: &str, value: &[u8]) -> StoredValue {
        StoredValue {
            primary_namespace: primary_namespace.to_string(),
            secondary_namespace: String::new(),
            key: key.to_string(),
            value: value.to_vec(),
        }
    }

    #[test]
    fn rejects_a_seed_of_the_wrong_length() {
        let err = build_force_close("00".repeat(32), vec![1; 32], vec![]).unwrap_err();
        assert_eq!(err, ForceCloseError::InvalidSeed);
    }

    #[test]
    fn finds_no_channel_in_an_empty_store() {
        let err = build_force_close(
            "00".repeat(32),
            vec![1; 64],
            vec![value("manager", "manager", &[1])],
        )
        .unwrap_err();
        assert_eq!(err, ForceCloseError::ChannelNotFound);
    }

    #[test]
    fn keeps_the_reason_of_unreadable_channel_data() {
        let err = build_force_close(
            "00".repeat(32),
            vec![1; 64],
            vec![value("monitors", "not_an_outpoint", &[1, 2, 3])],
        )
        .unwrap_err();
        assert!(
            matches!(&err, ForceCloseError::ReadFailed { reason } if !reason.is_empty()),
            "{err:?}"
        );
    }

    #[test]
    fn stored_values_are_read_only() {
        let store = StoredValues(HashMap::from([(
            ("monitors".to_string(), String::new()),
            HashMap::from([("a_0".to_string(), vec![7])]),
        )]));
        assert_eq!(store.read("monitors", "", "a_0").unwrap(), vec![7]);
        assert_eq!(
            store.read("monitors", "", "b_0").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(store.list("monitors", "").unwrap(), vec!["a_0".to_string()]);
        assert!(store.list("monitor_updates", "a_0").unwrap().is_empty());
        assert!(store.write("monitors", "", "a_0", vec![]).is_err());
        assert!(store.remove("monitors", "", "a_0", false).is_err());
    }
}
