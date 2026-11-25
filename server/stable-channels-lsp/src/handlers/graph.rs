//! REST proxies for LDK Server network-graph endpoints.

use axum::body::Bytes;
use axum::Extension;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    GraphGetChannelRequest, GraphGetNodeRequest, GraphListChannelsRequest, GraphListNodesRequest,
};

use crate::access::sessions::Caller;
use crate::handlers::{decode_body, map_grpc_error, ok_response};

pub async fn graph_list_channels(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: GraphListChannelsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.graph_list_channels(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn graph_get_channel(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: GraphGetChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.graph_get_channel(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn graph_list_nodes(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: GraphListNodesRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.graph_list_nodes(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn graph_get_node(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: GraphGetNodeRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.graph_get_node(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
