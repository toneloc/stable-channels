//! The daemon's LDK Server client: every call gives up after a fixed time, so one call that never returns can't hold the manager lock or a route forever.

use std::future::Future;
use std::time::Duration;

use ldk_server_client::client::{EventStream, LdkServerClient};
use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
use ldk_server_client::ldk_server_grpc::api::*;

/// Generous on purpose: a send that times out may still go out, and a top-up whose send never reached LDK Server waits for the operator.
pub(crate) const LDK_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The only handle the daemon has on LDK Server; the raw client is never handed out, so no call can skip the limit.
pub struct LdkClient {
    inner: LdkServerClient,
    limit: Duration,
}

impl LdkClient {
    pub fn new(inner: LdkServerClient) -> Self {
        Self::with_limit(inner, LDK_CALL_TIMEOUT)
    }

    fn with_limit(inner: LdkServerClient, limit: Duration) -> Self {
        Self { inner, limit }
    }

    /// Bounded only until the stream opens: an open stream can stay quiet for hours.
    pub async fn subscribe_events(&self) -> Result<EventStream, LdkServerError> {
        within(self.limit, "subscribe_events", self.inner.subscribe_events()).await
    }
}

/// Whether asking again may get another answer: LDK Server timed out, wasn't reached or failed inside, rather than refusing the call.
pub(crate) fn unanswered(error: &LdkServerError) -> bool {
    !matches!(
        error.error_code,
        LdkServerErrorCode::LightningError | LdkServerErrorCode::InvalidRequestError | LdkServerErrorCode::AuthError
    )
}

/// A call past the limit fails with InternalError, never with a code that says LDK Server refused it, since it may still complete.
async fn within<T>(
    limit: Duration,
    method: &str,
    call: impl Future<Output = Result<T, LdkServerError>>,
) -> Result<T, LdkServerError> {
    tokio::time::timeout(limit, call).await.unwrap_or_else(|_| {
        Err(LdkServerError::new(
            LdkServerErrorCode::InternalError,
            format!("{method} got no answer from LDK Server within {limit:?}; it may still complete"),
        ))
    })
}

macro_rules! bounded_calls {
    ($($method:ident: $request:ident => $response:ident,)*) => {
        impl LdkClient {
            $(pub async fn $method(&self, request: $request) -> Result<$response, LdkServerError> {
                within(self.limit, stringify!($method), self.inner.$method(request)).await
            })*
        }
    };
}

bounded_calls! {
    bolt11_receive: Bolt11ReceiveRequest => Bolt11ReceiveResponse,
    bolt11_send: Bolt11SendRequest => Bolt11SendResponse,
    bolt12_receive: Bolt12ReceiveRequest => Bolt12ReceiveResponse,
    bolt12_send: Bolt12SendRequest => Bolt12SendResponse,
    close_channel: CloseChannelRequest => CloseChannelResponse,
    connect_peer: ConnectPeerRequest => ConnectPeerResponse,
    disconnect_peer: DisconnectPeerRequest => DisconnectPeerResponse,
    export_pathfinding_scores: ExportPathfindingScoresRequest => ExportPathfindingScoresResponse,
    force_close_channel: ForceCloseChannelRequest => ForceCloseChannelResponse,
    get_balances: GetBalancesRequest => GetBalancesResponse,
    get_forwarded_payment_tracking_mode: GetForwardedPaymentTrackingModeRequest => GetForwardedPaymentTrackingModeResponse,
    get_node_info: GetNodeInfoRequest => GetNodeInfoResponse,
    get_payment_details: GetPaymentDetailsRequest => GetPaymentDetailsResponse,
    graph_get_channel: GraphGetChannelRequest => GraphGetChannelResponse,
    graph_get_node: GraphGetNodeRequest => GraphGetNodeResponse,
    graph_list_channels: GraphListChannelsRequest => GraphListChannelsResponse,
    graph_list_nodes: GraphListNodesRequest => GraphListNodesResponse,
    list_channels: ListChannelsRequest => ListChannelsResponse,
    list_forwarded_payments: ListForwardedPaymentsRequest => ListForwardedPaymentsResponse,
    list_payments: ListPaymentsRequest => ListPaymentsResponse,
    list_peers: ListPeersRequest => ListPeersResponse,
    onchain_receive: OnchainReceiveRequest => OnchainReceiveResponse,
    onchain_send: OnchainSendRequest => OnchainSendResponse,
    open_channel: OpenChannelRequest => OpenChannelResponse,
    sign_message: SignMessageRequest => SignMessageResponse,
    splice_in: SpliceInRequest => SpliceInResponse,
    splice_out: SpliceOutRequest => SpliceOutResponse,
    spontaneous_send: SpontaneousSendRequest => SpontaneousSendResponse,
    update_channel_config: UpdateChannelConfigRequest => UpdateChannelConfigResponse,
    verify_signature: VerifySignatureRequest => VerifySignatureResponse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stable_manager::LdkServerCalls;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Accepts connections and never answers, like LDK Server stuck inside a call or a path that drops packets.
    async fn silent_server() -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&accepted);
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                held.push(socket);
            }
        });
        (address, accepted)
    }

    fn client_for(address: String, limit: Duration) -> LdkClient {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let storage = tempfile::tempdir().unwrap();
        crate::tls::get_or_generate_tls_config(None, storage.path().to_str().unwrap()).unwrap();
        let cert = std::fs::read(storage.path().join("tls.crt")).unwrap();
        LdkClient::with_limit(LdkServerClient::new(address, "00".repeat(32), &cert).unwrap(), limit)
    }

    #[tokio::test]
    async fn a_call_ldk_server_never_answers_fails_after_the_limit_as_an_internal_error() {
        let (address, accepted) = silent_server().await;
        let client = client_for(address, Duration::from_millis(200));
        let started = std::time::Instant::now();
        let error = tokio::time::timeout(Duration::from_secs(10), client.list_channels(ListChannelsRequest {}))
            .await
            .expect("the client gives up by itself")
            .unwrap_err();
        assert_eq!(error.error_code, LdkServerErrorCode::InternalError);
        assert!(error.message.contains("list_channels"), "{}", error.message);
        assert!(started.elapsed() >= Duration::from_millis(200));
        assert!(accepted.load(Ordering::SeqCst) >= 1, "the call reached the server and waited there");
        // The manager, the tick and the event loop reach it through the trait.
        let ldk: &dyn LdkServerCalls = &client;
        let error = tokio::time::timeout(Duration::from_secs(10), ldk.spontaneous_send(SpontaneousSendRequest::default()))
            .await
            .expect("the client gives up by itself")
            .unwrap_err();
        assert_eq!(error.error_code, LdkServerErrorCode::InternalError);
    }

    #[tokio::test]
    async fn opening_the_event_stream_is_bounded_too() {
        let (address, _) = silent_server().await;
        let client = client_for(address, Duration::from_millis(200));
        let error = tokio::time::timeout(Duration::from_secs(10), client.subscribe_events())
            .await
            .expect("the client gives up by itself")
            .err()
            .expect("no stream from a server that never answers");
        assert_eq!(error.error_code, LdkServerErrorCode::InternalError);
        assert!(error.message.contains("subscribe_events"), "{}", error.message);
    }

    #[tokio::test(start_paused = true)]
    async fn the_limit_is_thirty_seconds() {
        let started = tokio::time::Instant::now();
        let answered = within(LDK_CALL_TIMEOUT, "get_balances", async {
            tokio::time::sleep(Duration::from_secs(29)).await;
            Ok::<_, LdkServerError>(7)
        })
        .await;
        assert_eq!(answered, Ok(7));
        let started_again = tokio::time::Instant::now();
        let error = within(LDK_CALL_TIMEOUT, "get_balances", std::future::pending::<Result<(), LdkServerError>>())
            .await
            .unwrap_err();
        assert_eq!(started_again - started, Duration::from_secs(29));
        assert_eq!(tokio::time::Instant::now() - started_again, Duration::from_secs(30));
        assert_eq!(error.error_code, LdkServerErrorCode::InternalError);
    }
}
