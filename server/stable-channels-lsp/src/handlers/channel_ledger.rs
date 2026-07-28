//! Structured, authenticated channel-ledger query endpoint.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;
use sc_protos::stable::{
    AccountingSnapshot as ProtoSnapshot, ChannelLedgerEvent, ChannelLedgerOverview,
    LedgerRef as ProtoRef, ListChannelLedgerEventsRequest, ListChannelLedgerEventsResponse,
};
use stable_channels::ledger::{
    AccountingSnapshot, LedgerCursor, LedgerEvent, LedgerOverview, LedgerQuery,
};

use crate::handlers::{decode_body, error_response, ok_response};
use crate::state::AppState;

pub async fn list_channel_ledger_events(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListChannelLedgerEventsRequest = match decode_body(&body) {
        Ok(req) => req,
        Err(response) => return response,
    };
    let Some(identifier) = nonempty(req.identifier) else {
        return error_response(
            ErrorCode::InvalidRequestError,
            "An exact ledger identifier is required",
        );
    };
    let before = if req.cursor.trim().is_empty() {
        None
    } else {
        match parse_cursor(&req.cursor) {
            Some(cursor) => Some(cursor),
            _ => return error_response(ErrorCode::InvalidRequestError, "Invalid ledger cursor"),
        }
    };
    if !req.completeness.is_empty()
        && !matches!(req.completeness.as_str(), "observed" | "reconstructed" | "legacy" | "gap")
    {
        return error_response(ErrorCode::InvalidRequestError, "Invalid completeness filter");
    }
    let query = LedgerQuery {
        identifier: Some(identifier),
        category: nonempty(req.category),
        status: nonempty(req.status),
        completeness: nonempty(req.completeness),
        before,
        limit: req.page_size as usize,
    };
    match state.db.list_ledger_events(&query) {
        Ok(page) => ok_response(ListChannelLedgerEventsResponse {
            events: page.events.into_iter().map(to_proto_event).collect(),
            next_cursor: page.next_cursor.map(format_cursor),
            overview: Some(to_proto_overview(page.overview)),
        }),
        Err(error) => error_response(
            ErrorCode::InternalServerError,
            format!("Failed to query channel ledger: {error}"),
        ),
    }
}

fn parse_cursor(raw: &str) -> Option<LedgerCursor> {
    let (occurred_at_ms, id) = raw.trim().split_once(':')?;
    let occurred_at_ms = occurred_at_ms.parse::<i64>().ok()?;
    let id = id.parse::<i64>().ok()?;
    (occurred_at_ms != 0 && id > 0).then_some(LedgerCursor { occurred_at_ms, id })
}

fn format_cursor(cursor: LedgerCursor) -> String {
    format!("{}:{}", cursor.occurred_at_ms, cursor.id)
}

fn to_proto_overview(overview: LedgerOverview) -> ChannelLedgerOverview {
    ChannelLedgerOverview {
        total_events: overview.total_events,
        matching_events: overview.matching_events,
        oldest_occurred_at_ms: overview.oldest_occurred_at_ms,
        newest_occurred_at_ms: overview.newest_occurred_at_ms,
        observed_events: overview.observed_events,
        reconstructed_events: overview.reconstructed_events,
        legacy_events: overview.legacy_events,
        gap_events: overview.gap_events,
        latest_accounting: overview.latest_accounting.map(to_proto_snapshot),
        latest_accounting_at_ms: overview.latest_accounting_at_ms,
        latest_accounting_source: overview.latest_accounting_source.unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_cursor_round_trips_and_rejects_legacy_ids() {
        let cursor = LedgerCursor { occurred_at_ms: 1_234, id: 56 };
        assert_eq!(parse_cursor(&format_cursor(cursor)), Some(cursor));
        assert_eq!(parse_cursor("56"), None);
        assert_eq!(parse_cursor("0:56"), None);
        assert_eq!(parse_cursor("1234:0"), None);
    }

    #[test]
    fn exact_identifier_must_not_be_empty() {
        assert_eq!(nonempty("  ".to_owned()), None);
        assert_eq!(
            nonempty("channel-1".to_owned()).as_deref(),
            Some("channel-1")
        );
    }
}
