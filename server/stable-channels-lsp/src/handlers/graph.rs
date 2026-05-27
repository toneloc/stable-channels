//! REST proxies for LDK Server network-graph endpoints.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    GraphGetChannelRequest, GraphGetNodeRequest, GraphListChannelsRequest, GraphListNodesRequest,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
use crate::state::AppState;

pub async fn graph_list_channels(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GraphListChannelsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.graph_list_channels(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn graph_get_channel(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GraphGetChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.graph_get_channel(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn graph_list_nodes(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GraphListNodesRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.graph_list_nodes(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn graph_get_node(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GraphGetNodeRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.graph_get_node(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
