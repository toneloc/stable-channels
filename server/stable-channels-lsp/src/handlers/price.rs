//! GetPrice handler. Reads the cached BTC/USD price from `stable_channels::price_feeds`.

use axum::body::Bytes;
use axum::response::Response;

use sc_protos::stable::{GetPriceRequest, GetPriceResponse};
use stable_channels::price_feeds::get_fresh_cached_price_no_fetch;

use crate::handlers::{decode_body, ok_response};

pub async fn get_price(body: Bytes) -> Response {
    if let Err(resp) = decode_body::<GetPriceRequest>(&body) {
        return resp;
    }
    let price = get_fresh_cached_price_no_fetch();
    ok_response(GetPriceResponse { price })
}
