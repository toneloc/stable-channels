use ldk_node::{bitcoin::secp256k1::PublicKey, config::ChannelConfig, lightning::ln::msgs::SocketAddress, payment::{Bolt11Payment, OnchainPayment, SpontaneousPayment}, BalanceDetails, ChannelDetails, Event, UserChannelId};

use std::fmt;

// I had to wrap this for some reason ... error internal
pub enum LightningError {
    LdkError(String), 
}

pub trait LightningNode: Send + Sync {
    // These are all ~read functions
    fn list_balances(&self) -> BalanceDetails;
    fn list_channels(&self) -> Vec<ChannelDetails>;

    fn node_id(&self) -> PublicKey;
    fn listening_addresses(&self) -> Option<Vec<SocketAddress>>;

    fn next_event(&self) -> Option<Event>;
    fn event_handled(&self);

    // These are all ~write functions
    fn open_announced_channel(
        &self,
        node_id: PublicKey,
        address: SocketAddress,
        channel_amount_sats: u64,
        push_amount_msat: Option<u64>,
        config: Option<ChannelConfig>,
    ) -> Result<UserChannelId, LightningError>;

    fn bolt11_payment(&self) -> Bolt11Payment;

    fn spontaneous_payment(&self) -> SpontaneousPayment;

    fn onchain_payment(&self) -> OnchainPayment;

    fn close_channel(
        &self,
        user_channel_id: &UserChannelId,
        counterparty_node_id: PublicKey,
    ) -> Result<(), LightningError>;

    fn force_close_channel(
        &self,
        user_channel_id: &ldk_node::UserChannelId,
        counterparty_node_id: ldk_node::bitcoin::secp256k1::PublicKey,
        reason: Option<String>,
    ) -> Result<(), LightningError>;


}

impl fmt::Display for LightningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LightningError::LdkError(s) => write!(f, "LDK error: {}", s),
        }
    }
}