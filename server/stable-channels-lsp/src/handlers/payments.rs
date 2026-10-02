//! REST proxies for LDK Server payment-history endpoints.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use std::collections::HashSet;

use ldk_server_client::ldk_server_grpc::api::{
    GetPaymentDetailsRequest, ListForwardedPaymentsRequest, ListPaymentsRequest,
    ListPaymentsResponse,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

/// A malformed or unusual upstream pagination implementation must not make this REST request
/// loop forever. A normal LDK Server history page is much smaller than this bound.
const MAX_PAYMENT_PAGES_PER_REQUEST: usize = 100;

/// List payments while hiding failed outbound 1-msat protocol messages.
///
/// The upstream token remains the cursor: when filtering leaves a page short, consume whole
/// upstream pages until the first page's capacity is filled (or the upstream history ends), then
/// return the last upstream `next_page_token`. Consuming whole pages avoids dropping visible rows
/// when a page crosses the fill target.
async fn list_visible_payments(
    ldk: &dyn LdkServerCalls,
    request: ListPaymentsRequest,
) -> Result<ListPaymentsResponse, ldk_server_client::error::LdkServerError> {
    let mut page_token = request.page_token;
    let mut seen_cursors: HashSet<String> = page_token.iter().cloned().collect();

    let mut payments = Vec::new();
    let mut target_page_len = None;
    let mut next_page_token = None;

    for _ in 0..MAX_PAYMENT_PAGES_PER_REQUEST {
        let response = ldk
            .list_payments(ListPaymentsRequest {
                page_token: page_token.clone(),
            })
            .await?;
        let upstream_next = response.next_page_token;
        let page_len = response.payments.len();

        // The first page's length is the upstream page size, or 1 when it came back empty with a cursor, so empty filtered pages are walked.
        let target = *target_page_len.get_or_insert(page_len.max(1));
        payments.extend(
            response
                .payments
                .into_iter()
                .filter(|payment| !crate::payment_filter::is_failed_protocol_message(payment)),
        );
        next_page_token = upstream_next.clone();
        let Some(next) = upstream_next.filter(|_| payments.len() < target) else { break };
        // Preserve the upstream cursor in the response, but stop if a broken upstream repeats it.
        if !seen_cursors.insert(next.clone()) {
            break;
        }
        page_token = Some(next);
    }

    Ok(ListPaymentsResponse {
        payments,
        next_page_token,
    })
}

pub async fn list_payments(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListPaymentsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match list_visible_payments(state.ldk_server.as_ref(), req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn get_payment_details(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GetPaymentDetailsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.get_payment_details(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn list_forwarded_payments(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListForwardedPaymentsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.list_forwarded_payments(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ldk_server_client::error::LdkServerError;
    use ldk_server_client::ldk_server_grpc::api::{
        GetBalancesRequest, GetBalancesResponse, GetForwardedPaymentTrackingModeRequest,
        GetForwardedPaymentTrackingModeResponse, GetPaymentDetailsResponse, ListChannelsRequest,
        ListChannelsResponse, ListPeersRequest, ListPeersResponse, SignMessageRequest,
        SignMessageResponse, SpontaneousSendRequest, SpontaneousSendResponse,
        VerifySignatureRequest, VerifySignatureResponse,
    };
    use ldk_server_client::ldk_server_grpc::types::{Payment, PaymentDirection, PaymentStatus};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakePayments {
        pages: Mutex<HashMap<Option<String>, ListPaymentsResponse>>,
        calls: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl LdkServerCalls for FakePayments {
        async fn list_channels(
            &self,
            _: ListChannelsRequest,
        ) -> Result<ListChannelsResponse, LdkServerError> {
            unreachable!()
        }
        async fn spontaneous_send(
            &self,
            _: SpontaneousSendRequest,
        ) -> Result<SpontaneousSendResponse, LdkServerError> {
            unreachable!()
        }
        async fn sign_message(
            &self,
            _: SignMessageRequest,
        ) -> Result<SignMessageResponse, LdkServerError> {
            unreachable!()
        }
        async fn verify_signature(
            &self,
            _: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            unreachable!()
        }
        async fn list_payments(
            &self,
            request: ListPaymentsRequest,
        ) -> Result<ListPaymentsResponse, LdkServerError> {
            self.calls.lock().unwrap().push(request.page_token.clone());
            Ok(self
                .pages
                .lock()
                .unwrap()
                .get(&request.page_token)
                .cloned()
                .unwrap_or_default())
        }
        async fn get_payment_details(
            &self,
            _: GetPaymentDetailsRequest,
        ) -> Result<GetPaymentDetailsResponse, LdkServerError> {
            unreachable!()
        }
        async fn get_balances(
            &self,
            _: GetBalancesRequest,
        ) -> Result<GetBalancesResponse, LdkServerError> {
            unreachable!()
        }
        async fn get_forwarded_payment_tracking_mode(
            &self,
            _: GetForwardedPaymentTrackingModeRequest,
        ) -> Result<GetForwardedPaymentTrackingModeResponse, LdkServerError> {
            unreachable!()
        }
        async fn list_peers(
            &self,
            _: ListPeersRequest,
        ) -> Result<ListPeersResponse, LdkServerError> {
            unreachable!()
        }
    }

    fn payment(
        id: &str,
        status: PaymentStatus,
        direction: PaymentDirection,
        amount: Option<u64>,
    ) -> Payment {
        Payment {
            payment_id: id.into(),
            status: status as i32,
            direction: direction as i32,
            amount_msat: amount,
            ..Default::default()
        }
    }

    fn response(payments: Vec<Payment>, next_page_token: Option<&str>) -> ListPaymentsResponse {
        ListPaymentsResponse {
            payments,
            next_page_token: next_page_token.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn list_payments_filters_only_failed_outbound_one_msat_rows() {
        let fake = FakePayments::default();
        fake.pages.lock().unwrap().insert(
            None,
            response(
                vec![
                    payment(
                        "protocol",
                        PaymentStatus::Failed,
                        PaymentDirection::Outbound,
                        Some(1),
                    ),
                    payment(
                        "failed-real",
                        PaymentStatus::Failed,
                        PaymentDirection::Outbound,
                        Some(2),
                    ),
                    payment(
                        "failed-inbound",
                        PaymentStatus::Failed,
                        PaymentDirection::Inbound,
                        Some(1),
                    ),
                    payment(
                        "success-one",
                        PaymentStatus::Succeeded,
                        PaymentDirection::Outbound,
                        Some(1),
                    ),
                ],
                None,
            ),
        );

        let listed = list_visible_payments(&fake, ListPaymentsRequest::default())
            .await
            .unwrap();

        assert_eq!(
            listed
                .payments
                .iter()
                .map(|p| p.payment_id.as_str())
                .collect::<Vec<_>>(),
            ["failed-real", "failed-inbound", "success-one"]
        );
        assert_eq!(listed.next_page_token, None);
    }

    #[tokio::test]
    async fn list_payments_fills_from_later_upstream_pages_and_keeps_cursor() {
        let fake = FakePayments::default();
        {
            let mut pages = fake.pages.lock().unwrap();
            pages.insert(
                None,
                response(
                    vec![
                        payment(
                            "protocol-1",
                            PaymentStatus::Failed,
                            PaymentDirection::Outbound,
                            Some(1),
                        ),
                        payment(
                            "protocol-2",
                            PaymentStatus::Failed,
                            PaymentDirection::Outbound,
                            Some(1),
                        ),
                    ],
                    Some("page-2"),
                ),
            );
            pages.insert(
                Some("page-2".into()),
                response(
                    vec![payment(
                        "visible-1",
                        PaymentStatus::Succeeded,
                        PaymentDirection::Outbound,
                        Some(5),
                    )],
                    Some("page-3"),
                ),
            );
            pages.insert(
                Some("page-3".into()),
                response(
                    vec![payment(
                        "visible-2",
                        PaymentStatus::Pending,
                        PaymentDirection::Inbound,
                        Some(7),
                    )],
                    Some("page-4"),
                ),
            );
        }

        let listed = list_visible_payments(&fake, ListPaymentsRequest::default())
            .await
            .unwrap();

        assert_eq!(
            listed
                .payments
                .iter()
                .map(|p| p.payment_id.as_str())
                .collect::<Vec<_>>(),
            ["visible-1", "visible-2"]
        );
        assert_eq!(listed.next_page_token.as_deref(), Some("page-4"));
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![None, Some("page-2".into()), Some("page-3".into())]
        );
    }

    #[tokio::test]
    async fn list_payments_stops_on_a_repeated_upstream_cursor() {
        let fake = FakePayments::default();
        fake.pages.lock().unwrap().insert(
            None,
            response(
                vec![payment(
                    "protocol",
                    PaymentStatus::Failed,
                    PaymentDirection::Outbound,
                    Some(1),
                )],
                Some("same"),
            ),
        );
        fake.pages.lock().unwrap().insert(
            Some("same".into()),
            response(
                vec![payment(
                    "protocol-2",
                    PaymentStatus::Failed,
                    PaymentDirection::Outbound,
                    Some(1),
                )],
                Some("same"),
            ),
        );

        let listed = list_visible_payments(&fake, ListPaymentsRequest::default())
            .await
            .unwrap();

        assert!(listed.payments.is_empty());
        assert_eq!(listed.next_page_token.as_deref(), Some("same"));
        assert_eq!(*fake.calls.lock().unwrap(), vec![None, Some("same".into())]);
    }
}
