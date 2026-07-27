//! Structured, authenticated channel-ledger query endpoint.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;
use sc_protos::stable::{
    AccountingSnapshot as ProtoSnapshot, ChannelLedgerEvent, LedgerRef as ProtoRef,
    ListChannelLedgerEventsRequest, ListChannelLedgerEventsResponse,
};
use stable_channels::ledger::{AccountingSnapshot, LedgerEvent, LedgerQuery};

use crate::handlers::{decode_body, error_response, ok_response};
use crate::state::AppState;

pub async fn list_channel_ledger_events(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListChannelLedgerEventsRequest = match decode_body(&body) {
        Ok(req) => req,
        Err(response) => return response,
    };
    let before_id = if req.cursor.trim().is_empty() {
        None
    } else {
        match req.cursor.parse::<i64>() {
            Ok(value) if value > 0 => Some(value),
            _ => return error_response(ErrorCode::InvalidRequestError, "Invalid ledger cursor"),
        }
    };
    if !req.completeness.is_empty()
        && !matches!(req.completeness.as_str(), "observed" | "reconstructed" | "legacy" | "gap")
    {
        return error_response(ErrorCode::InvalidRequestError, "Invalid completeness filter");
    }
    let query = LedgerQuery {
        identifier: nonempty(req.identifier),
        category: nonempty(req.category),
        status: nonempty(req.status),
        completeness: nonempty(req.completeness),
        before_id,
        limit: req.page_size as usize,
    };
    match state.db.list_ledger_events(&query) {
        Ok(page) => ok_response(ListChannelLedgerEventsResponse {
            events: page.events.into_iter().map(to_proto_event).collect(),
            next_cursor: page.next_cursor.map(|cursor| cursor.to_string()),
        }),
        Err(error) => error_response(
            ErrorCode::InternalServerError,
            format!("Failed to query channel ledger: {error}"),
        ),
    }
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn to_proto_event(event: LedgerEvent) -> ChannelLedgerEvent {
    ChannelLedgerEvent {
        id: event.id,
        event_type: event.event_type,
        category: event.category,
        severity: event.severity,
        status: event.status,
        source: event.source,
        completeness: event.completeness.as_str().to_owned(),
        occurred_at_ms: event.occurred_at_ms,
        recorded_at_ms: event.recorded_at_ms,
        dedup_key: event.dedup_key,
        before: event.before.map(to_proto_snapshot),
        after: event.after.map(to_proto_snapshot),
        detail_json: event.detail.to_string(),
        refs: event.refs.into_iter().map(|reference| ProtoRef {
            role: reference.role,
            value: reference.value,
        }).collect(),
    }
}

fn to_proto_snapshot(snapshot: AccountingSnapshot) -> ProtoSnapshot {
    ProtoSnapshot {
        expected_usd: snapshot.expected_usd,
        backing_sats: snapshot.backing_sats,
        native_sats: snapshot.native_sats,
        live_receiver_sats: snapshot.live_receiver_sats,
        btc_price: snapshot.btc_price,
        amount_sats: snapshot.amount_sats,
        amount_msat: snapshot.amount_msat,
        amount_usd: snapshot.amount_usd,
        fee_sats: snapshot.fee_sats,
        fee_msat: snapshot.fee_msat,
    }
}
