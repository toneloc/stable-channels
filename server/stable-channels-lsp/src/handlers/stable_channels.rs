//! ListStableChannels handler. Reads from the in-memory StableChannelManager.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{
    EditStableChannelRequest, EditStableChannelResponse, ListSettlementPaymentsRequest,
    ListSettlementPaymentsResponse, ListStableChannelsRequest, ListStableChannelsResponse,
    ReleaseStabilityPaymentRequest, ReleaseStabilityPaymentResponse, SettlementPayment,
    StableChannelInfo,
};
use stable_channels::price_feeds::get_fresh_cached_price_no_fetch;

use crate::handlers::{decode_body, error_response, ok_response};
use crate::stable_manager::{EditOutcome, ReleaseOutcome};
use crate::state::AppState;

fn to_proto_locations(rows: Vec<stable_channels::db::PeerLocationRecord>) -> Vec<sc_protos::stable::PeerLocation> {
    rows.into_iter()
        .map(|row| sc_protos::stable::PeerLocation {
            ip: row.ip,
            country_code: row.country_code.unwrap_or_default(),
            country_name: row.country_name.unwrap_or_default(),
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
        })
        .collect()
}

/// A stable user's latest sightings; none while recording is off, whatever an earlier run stored.
fn recent_locations(db: &stable_channels::db::Database, record_ips: bool, node_id: &str) -> Vec<sc_protos::stable::PeerLocation> {
    if !record_ips {
        return Vec::new();
    }
    to_proto_locations(db.recent_peer_locations(node_id, 10).unwrap_or_default())
}

pub async fn list_stable_channels(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if let Err(resp) = decode_body::<ListStableChannelsRequest>(&body) {
        return resp;
    }

    let latest_price = get_fresh_cached_price_no_fetch();
    // One read for every row; a missing entry reports 0 so old readers see nothing new.
    let created_at = state.db.channel_created_at().unwrap_or_default();

    let mgr = state.stable_manager.lock().await;
    let channels = mgr
        .stable_channels
        .iter()
        .map(|sc| StableChannelInfo {
            channel_id: sc.channel_id.to_string(),
            counterparty: sc.counterparty.to_string(),
            expected_usd: sc.expected_usd.0,
            expected_msats: sc.backing_sats.saturating_mul(1_000),
            latest_price,
            note: sc.note.clone().unwrap_or_default(),
            is_stable_receiver: sc.is_stable_receiver,
            user_channel_id: format!("{}", sc.user_channel_id),
            recent_locations: recent_locations(&state.db, state.geoip_record_ips, &sc.counterparty.to_string()),
            created_at: created_at.get(&sc.user_channel_id.to_string()).copied().unwrap_or(0),
        })
        .collect::<Vec<_>>();
    drop(mgr);

    ok_response(ListStableChannelsResponse { channels })
}

pub async fn edit_stable_channel(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let req: EditStableChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let btc_price = get_fresh_cached_price_no_fetch();

    let EditOutcome { ok, status } = {
        let mut mgr = state.stable_manager.lock().await;
        mgr.edit_stable_channel(
            &req.channel_id,
            req.expected_usd,
            req.note,
            state.ldk_server.as_ref() as &dyn crate::stable_manager::LdkServerCalls,
            btc_price,
        )
        .await
    };

    ok_response(EditStableChannelResponse { ok, status })
}

/// Whether a release counts the payment as arrived; None for anything but the two spelled-out answers, so an empty request can never drop a payment.
fn release_decision(decision: &str) -> Option<bool> {
    match decision {
        "arrived" => Some(true),
        "not_arrived" => Some(false),
        _ => None,
    }
}

/// Operator's decision about a stability payment the node has no record of.
pub async fn release_stability_payment(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let req: ReleaseStabilityPaymentRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let Some(arrived) = release_decision(&req.decision) else {
        let status = format!("Unknown decision {:?}; send \"arrived\" or \"not_arrived\"", req.decision);
        return ok_response(ReleaseStabilityPaymentResponse { ok: false, status, needs_acknowledgement: false });
    };
    let btc_price = get_fresh_cached_price_no_fetch();

    let ReleaseOutcome { ok, status, needs_acknowledgement } = {
        let mut mgr = state.stable_manager.lock().await;
        mgr.release_stability_payment(
            &req.payment_id,
            arrived,
            req.acknowledged,
            state.ldk_server.as_ref() as &dyn crate::stable_manager::LdkServerCalls,
            btc_price,
        )
        .await
    };

    ok_response(ReleaseStabilityPaymentResponse { ok, status, needs_acknowledgement })
}

pub async fn list_settlement_payments(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if let Err(resp) = decode_body::<ListSettlementPaymentsRequest>(&body) {
        return resp;
    }

    let rows = match state.db.list_settlements() {
        Ok(r) => r,
        Err(e) => {
            return error_response(
                ldk_server_client::ldk_server_grpc::error::ErrorCode::InternalServerError,
                format!("list_settlements failed: {}", e),
            )
        }
    };

    let settlements = rows
        .into_iter()
        .map(|(payment_id, kind)| SettlementPayment { payment_id, kind })
        .collect::<Vec<_>>();

    ok_response(ListSettlementPaymentsResponse { settlements })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use stable_channels::db::PeerLocationRecord;

    #[test]
    fn release_needs_a_spelled_out_decision() {
        assert_eq!(release_decision("arrived"), Some(true));
        assert_eq!(release_decision("not_arrived"), Some(false));
        for unclear in ["", "Arrived", "true", "false", "not arrived"] {
            assert_eq!(release_decision(unclear), None, "{unclear:?}");
        }
    }

    #[test]
    fn locations_map_to_proto_and_old_readers_ignore_them() {
        let rows = vec![PeerLocationRecord {
            ip: "1.2.3.4".into(),
            country_code: Some("IN".into()),
            country_name: Some("India".into()),
            first_seen_at: 10,
            last_seen_at: 20,
        }];
        let info = StableChannelInfo { channel_id: "c".into(), recent_locations: to_proto_locations(rows), ..Default::default() };
        assert_eq!(info.recent_locations[0].country_code, "IN");
        assert_eq!(info.recent_locations[0].last_seen_at, 20);
        // An old reader's struct is the same message without field 9: decoding must still succeed.
        #[derive(Clone, PartialEq, prost::Message)]
        struct OldInfo {
            #[prost(string, tag = "1")]
            channel_id: String,
        }
        assert_eq!(OldInfo::decode(info.encode_to_vec().as_slice()).unwrap().channel_id, "c");
    }

    #[test]
    fn no_locations_are_served_while_recording_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        db.record_peer_sighting("wallet", "1.2.3.4", None, 10).unwrap();
        assert_eq!(recent_locations(&db, true, "wallet").len(), 1);
        assert!(recent_locations(&db, false, "wallet").is_empty());
    }
}
