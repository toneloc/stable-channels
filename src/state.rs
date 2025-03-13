use crate::price_feeds::{calculate_median_price, fetch_prices, set_price_feeds};
use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::{
    bitcoin::secp256k1::PublicKey,
    lightning::ln::types::ChannelId,
    ChannelDetails, Node,
};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use ureq::Agent;

/// Represents the action to take after a stability check
#[derive(Debug, Clone)]
pub enum StabilityAction {
    /// No action needed, channel is stable enough
    DoNothing,
    /// Wait for payment from counterparty
    Wait,
    /// Make a payment to maintain stability
    Pay(u64), // amount in msats
    /// High risk situation detected
    HighRisk(u32), // risk level
    /// Channel not properly initialized or not found
    NotInitialized,
}

/// Main state management for Stable Channels
pub struct StateManager {
    /// LDK Node instance
    node: Arc<Node>,
    /// Current stable channel state
    stable_channel: Arc<Mutex<StableChannel>>,
    /// HTTP agent for price fetching
    agent: Agent,
    /// Last timestamp of stability check
    last_check: Arc<Mutex<SystemTime>>,
    /// Whether the channel has been properly initialized
    initialized: Arc<Mutex<bool>>,
}

impl StateManager {
    /// Create a new state manager with the given node
    pub fn new(node: Node) -> Self {
        Self {
            node: Arc::new(node),
            stable_channel: Arc::new(Mutex::new(StableChannel::default())),
            agent: Agent::new(),
            last_check: Arc::new(Mutex::new(SystemTime::now())),
            initialized: Arc::new(Mutex::new(false)),
        }
    }

    /// Get a reference to the node
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// Check if the state manager has been properly initialized with a valid channel
    pub fn is_initialized(&self) -> bool {
        *self.initialized.lock().unwrap()
    }

    /// Get the current stable channel state
    pub fn get_stable_channel(&self) -> StableChannel {
        self.stable_channel.lock().unwrap().clone()
    }

    /// Initialize a stable channel with the given parameters
    pub fn initialize_stable_channel(
        &self,
        channel_id_str: &str,
        is_stable_receiver: bool,
        expected_dollar_amount: f64,
        native_amount_sats: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sc = self.stable_channel.lock().unwrap();
        
        // Check if the channel_id is provided as hex string or full channel id
        let channel_id = if channel_id_str.len() == 64 { // It's a hex string
            let channel_id_bytes: [u8; 32] = hex::decode(channel_id_str)?
                .try_into()
                .map_err(|_| "Decoded channel ID has incorrect length")?;
            ChannelId::from_bytes(channel_id_bytes)
        } else { // It's already a formatted channel id
            ChannelId::from_str(channel_id_str)?
        };

        // Find the counterparty node ID from the channel list
        let mut counterparty = None;
        for channel in self.node.list_channels() {
            if channel.channel_id.to_string() == channel_id.to_string() {
                counterparty = Some(channel.counterparty_node_id);
                break;
            }
        }

        let counterparty = counterparty.ok_or("Failed to find channel with the specified ID")?;

        // Update the stable channel state
        sc.channel_id = channel_id;
        sc.is_stable_receiver = is_stable_receiver;
        sc.counterparty = counterparty;
        sc.expected_usd = USD::from_f64(expected_dollar_amount);
        sc.expected_btc = Bitcoin::from_btc(native_amount_sats);
        
        // Get initial price
        let latest_price = self.get_latest_price();
        sc.latest_price = latest_price;

        // Update balances
        drop(sc); // Release lock before calling update_balances
        self.update_balances();

        // Mark as initialized only if the channel exists and balances were updated
        let mut initialized = self.initialized.lock().unwrap();
        *initialized = true;

        Ok(())
    }

    /// Fetch the latest BTC/USD price
    pub fn get_latest_price(&self) -> f64 {
        match fetch_prices(&self.agent, &set_price_feeds())
            .and_then(|prices| calculate_median_price(prices)) {
            Ok(price) => price,
            Err(e) => {
                eprintln!("Error fetching price: {:?}", e);
                60000.0 // Default fallback price
            }
        }
    }
    
    /// Check if the given channel exists in the node's channel list
    fn channel_exists(&self, channel_id: &ChannelId) -> bool {
        let channels = self.node.list_channels();
        channels.iter().any(|c| c.channel_id == *channel_id)
    }
    
    /// Update stable channel balances based on current channel state
    pub fn update_balances(&self) -> bool {
        let mut sc = self.stable_channel.lock().unwrap();
        
        // Find the matching channel
        let mut matching_channel_found = false;
        
        // Get current price if we don't have it
        if sc.latest_price == 0.0 {
            sc.latest_price = self.get_latest_price();
        }
        
        // First check if we're using the default channel ID (all zeros)
        let is_default = sc.channel_id == ChannelId::from_bytes([0; 32]);
        
        // If it's a default channel ID, try to find any available channel
        if is_default {
            if let Some(channel) = self.node.list_channels().first() {
                sc.channel_id = channel.channel_id;
                println!("Set active channel ID to: {}", sc.channel_id);
                matching_channel_found = true;
                
                // Update the channel balances
                let (our_balance, their_balance) = {
                    let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                    let our_balance_sats =
                        (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                    let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                    (our_balance_sats, their_balance_sats)
                };
                
                // Update balances based on whether we're the stable receiver or provider
                if sc.is_stable_receiver {
                    sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
                    sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                    sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
                    sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                } else {
                    sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
                    sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                    sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
                    sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                }
            }
        } else {
            // Otherwise, look for a channel matching our stored ID
            for channel in self.node.list_channels() {
                if channel.channel_id == sc.channel_id {
                    matching_channel_found = true;
                    
                    // Update the channel balances
                    let (our_balance, their_balance) = {
                        let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                        let our_balance_sats =
                            (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                        let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                        (our_balance_sats, their_balance_sats)
                    };
                    
                    // Update balances based on whether we're the stable receiver or provider
                    if sc.is_stable_receiver {
                        sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
                        sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                        sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
                        sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                    } else {
                        sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
                        sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                        sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
                        sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                    }
                    
                    break;
                }
            }
        }
        
        if !matching_channel_found {
            println!("No matching channel found for ID: {}", sc.channel_id);
            // If we can't find the channel, update initialization status
            if !is_default {
                println!("Channel may have been closed or not properly initialized.");
                if *self.initialized.lock().unwrap() {
                    *self.initialized.lock().unwrap() = false;
                }
            }
        }
        
        // Update last check time
        *self.last_check.lock().unwrap() = SystemTime::now();
        
        matching_channel_found
    }
    
    /// Check if the stable channel is in balance and determine what action to take
    pub fn check_stability(&self) -> StabilityAction {
        // First, check if we're initialized with a valid channel
        let is_initialized = *self.initialized.lock().unwrap();
        if !is_initialized {
            let default_channel_id = ChannelId::from_bytes([0; 32]);
            
            // Get the current stable channel
            let sc = self.stable_channel.lock().unwrap();
            
            // If we have a non-default channel ID but it's not found in the channels list
            if sc.channel_id != default_channel_id && !self.channel_exists(&sc.channel_id) {
                println!("Stable channel with ID {} not found or not properly initialized", sc.channel_id);
                return StabilityAction::NotInitialized;
            }
            
            // If we have an default channel ID and there are no channels
            if sc.channel_id == default_channel_id && self.node.list_channels().is_empty() {
                println!("No channels available. Please create a channel first.");
                return StabilityAction::NotInitialized;
            }
            
            // If we have a default channel ID but channels exist, we might be able to use one
            if sc.channel_id == default_channel_id && !self.node.list_channels().is_empty() {
                // Try to update balances and automatically select an available channel
                let balances_updated = self.update_balances();
                if balances_updated {
                    // If we successfully found a channel, mark as initialized
                    *self.initialized.lock().unwrap() = true;
                } else {
                    println!("Failed to initialize with available channels.");
                    return StabilityAction::NotInitialized;
                }
            }
        }
        
        // Update the price and balances
        let price = self.get_latest_price();
        
        {
            let mut sc = self.stable_channel.lock().unwrap();
            sc.latest_price = price;
        }
        
        // Try to update balances, return NotInitialized if failed
        if !self.update_balances() {
            return StabilityAction::NotInitialized;
        }
        
        let sc = self.stable_channel.lock().unwrap();
        
        // Print the current state
        println!("{:<25} ${:>15.2}", "BTC/USD Price:", sc.latest_price);
        println!("{:<25} {:>15}", "Expected USD:", sc.expected_usd);
        println!("{:<25} {:>15}", "User USD:", sc.stable_receiver_usd);
        
        // Check for division by zero - if expected_usd is 0, we can't calculate difference
        if sc.expected_usd.0 == 0.0 {
            println!("Expected USD amount is zero. Cannot calculate stability difference.");
            return StabilityAction::NotInitialized;
        }
        
        // Calculate difference from expected value
        let dollars_from_par: USD = sc.stable_receiver_usd - sc.expected_usd;
        let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();
        
        println!("{:<25} {:>5}", "Percent from par:", format!("{:.2}%", percent_from_par));
        println!("{:<25} {:>15}", "User BTC:", sc.stable_receiver_btc);
        println!("{:<25} {:>15}", "LSP USD:", sc.stable_provider_usd);
        println!("{:<25} {:>15}", "LSP BTC:", sc.stable_provider_btc);
        
        // Determine action based on conditions
        if percent_from_par < 0.1 {
            println!("\nDifference from par less than 0.1%. Doing nothing.");
            return StabilityAction::DoNothing;
        } 
        
        let is_receiver_below_expected = sc.stable_receiver_usd < sc.expected_usd;

        match (sc.is_stable_receiver, is_receiver_below_expected, sc.risk_level > 100) {
            (_, _, true) => {
                println!("Risk level high. Current risk level: {}", sc.risk_level);
                StabilityAction::HighRisk(sc.risk_level as u32)
            },
            (true, true, false) => {
                println!("\nWaiting for payment from counterparty...");
                StabilityAction::Wait
            },
            (true, false, false) => {
                println!("\nPaying the difference...");
                let amt = USD::to_msats(dollars_from_par, sc.latest_price);
                StabilityAction::Pay(amt)
            },
            (false, true, false) => {
                println!("\nPaying the difference...");
                let amt = USD::to_msats(dollars_from_par, sc.latest_price);
                StabilityAction::Pay(amt)
            },
            (false, false, false) => {
                println!("\nWaiting for payment from counterparty...");
                StabilityAction::Wait
            },
        }
    }
    
    /// Execute a payment to maintain stability
    pub fn execute_payment(&self, amount_msats: u64) -> Result<String, Box<dyn std::error::Error>> {
        // First check if we're initialized
        if !*self.initialized.lock().unwrap() {
            return Err("Stable channel not initialized".into());
        }
        
        let sc = self.stable_channel.lock().unwrap();
        
        // Verify the counterparty exists
        if !self.node.list_channels().iter().any(|c| c.counterparty_node_id == sc.counterparty) {
            return Err("Counterparty not found in available channels".into());
        }
        
        let result = self.node
            .spontaneous_payment()
            .send(amount_msats, sc.counterparty, None)?;
            
        Ok(result.to_string())
    }
    
    /// Get the time elapsed since the last stability check
    pub fn time_since_last_check(&self) -> Duration {
        SystemTime::now()
            .duration_since(*self.last_check.lock().unwrap())
            .unwrap_or(Duration::from_secs(0))
    }
}

/// Extension trait to add ChannelId::from_str
trait ChannelIdExt {
    fn from_str(s: &str) -> Result<ChannelId, Box<dyn std::error::Error>>;
}

impl ChannelIdExt for ChannelId {
    fn from_str(s: &str) -> Result<ChannelId, Box<dyn std::error::Error>> {
        // Simplified parsing - may need to be expanded based on the actual string format
        let clean_str = s.trim();
        
        if clean_str.len() >= 64 {
            // It's likely a hex string
            let hex_part = if clean_str.len() > 64 {
                // Extract just the 64 hex chars if there's extra formatting
                let start = clean_str.find(|c: char| c.is_ascii_hexdigit())
                    .ok_or("No hex digits found in channel ID string")?;
                &clean_str[start..(start+64)]
            } else {
                clean_str
            };
            
            let bytes = hex::decode(hex_part)?;
            if bytes.len() != 32 {
                return Err(format!("Expected 32 bytes, got {}", bytes.len()).into());
            }
            
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(ChannelId::from_bytes(arr))
        } else {
            Err("Channel ID string is too short".into())
        }
    }
}