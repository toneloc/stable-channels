use std::sync::Arc;
use ldk_node::{
    bitcoin::secp256k1::PublicKey, config::ChannelConfig, lightning::ln::msgs::SocketAddress, payment::{Bolt11Payment, OnchainPayment, SpontaneousPayment}, BalanceDetails, ChannelDetails, Event, Node, UserChannelId
};
use crate::lightning::{LightningError, LightningNode};

pub struct LdkNodeAdapter(pub Arc<Node>);

impl LightningNode for LdkNodeAdapter {
    // These are all ~read functions
    fn node_id(&self) -> ldk_node::bitcoin::secp256k1::PublicKey {
        self.0.node_id()
    }

    fn listening_addresses(&self) -> Option<Vec<SocketAddress>> {
        self.0.listening_addresses()
    }

    fn list_balances(&self) -> BalanceDetails {
        self.0.list_balances()
    }

    fn list_channels(&self) -> Vec<ChannelDetails> {
        self.0.list_channels()
    }

    fn next_event(&self) -> Option<Event> {
        self.0.next_event()
    }
    
    fn event_handled(&self) {
        let _ = self.0.event_handled();
    }
    
    // These are all ~write functions
    fn spontaneous_payment(&self) -> SpontaneousPayment {
        self.0.spontaneous_payment()
    }

    fn bolt11_payment(&self) -> Bolt11Payment {
        self.0.bolt11_payment()
    }

    fn onchain_payment(&self) -> OnchainPayment {
        self.0.onchain_payment()
    }

    fn force_close_channel(
        &self,
        user_channel_id: &UserChannelId,
        counterparty_node_id: PublicKey,
        reason: Option<String>,
    ) -> Result<(), LightningError> {
        self.0
            .force_close_channel(user_channel_id, counterparty_node_id, reason)
            .map_err(|e| LightningError::LdkError(e.to_string()))
    }

    fn open_announced_channel(
        &self,
        node_id: PublicKey,
        address: SocketAddress,
        channel_amount_sats: u64,
        push_amount_msat: Option<u64>,
        config: Option<ChannelConfig>,
    ) -> Result<UserChannelId, LightningError> {
        self.0
            .open_announced_channel(node_id, address, channel_amount_sats, push_amount_msat, config)
            .map_err(|e| LightningError::LdkError(e.to_string()))
    }

    fn close_channel(
        &self,
        user_channel_id: &UserChannelId,
        counterparty_node_id: PublicKey,
    ) -> Result<(), LightningError> {
        self.0
            .close_channel(user_channel_id, counterparty_node_id)
            .map_err(|e| LightningError::LdkError(e.to_string()))
    }
}