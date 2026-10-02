use ldk_server_client::ldk_server_grpc::types::{Payment, PaymentDirection, PaymentStatus};

/// A failed outbound 1-msat keysend carries a protocol message rather than a user payment.
pub(crate) fn is_failed_protocol_message(payment: &Payment) -> bool {
    payment.status == PaymentStatus::Failed as i32
        && payment.direction == PaymentDirection::Outbound as i32
        && payment.amount_msat == Some(1)
}
