use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};

use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::bitcoin::{Address, Network};
use ldk_node::config::ChannelConfig;
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning::ln::types::ChannelId;
use ldk_node::lightning::offers::offer::Offer;
use ldk_node::lightning_invoice::{
    Bolt11Invoice, Bolt11InvoiceDescription, Description
};
use ldk_node::{Node, Event, Builder};

use stable_channels::{StateManager, StabilityAction};
use crate::types::{Bitcoin, StableChannel, USD};
use crate::get_user_input;

const USER_DATA_DIR: &str = "data/user";
const USER_NODE_ALIAS: &str = "user";
const USER_PORT: u16 = 9736;
const DEFAULT_NETWORK: &str = "signet";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";
const DEFAULT_LSP_PUBKEY: &str = "022814b30dc90b3c53312c250021165644fdf1650aa7ba4be5d6cd51302b2f31bb";
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
const DEFAULT_LSP_AUTH: &str = "00000000000000000000000000000000";
const DEFAULT_EXPECTED_USD: f64 = 20.0;

struct UserState {
    node: Node,
    stable_channel: StableChannel,
    last_check: SystemTime,
    initialized: bool,
}

// GUI-specific imports
use eframe::{egui, App, Frame};
use egui::{epaint::{self, Margin}, TextureHandle, TextureOptions};
use image::{GrayImage, Luma};
use qrcode::{Color, QrCode};

#[cfg(feature = "user")]
fn make_user_node() -> Node {
    println!("Initializing user node...");

    let mut builder = Builder::new();
    
    // Parse LSP pubkey if available
    let lsp_pubkey = if !DEFAULT_LSP_PUBKEY.is_empty() {
        match hex::decode(DEFAULT_LSP_PUBKEY) {
            Ok(bytes) => {
                match PublicKey::from_slice(&bytes) {
                    Ok(key) => {
                        println!("Setting LSP pubkey: {}", key);
                        Some(key)
                    },
                    Err(e) => {
                        println!("Error parsing LSP pubkey: {:?}", e);
                        None
                    }
                }
            },
            Err(e) => {
                println!("Error decoding LSP pubkey: {:?}", e);
                None
            }
        }
    } else {
        None
    };
    
    // Configure LSP if pubkey is available
    if let Some(lsp_pubkey) = lsp_pubkey {
        let lsp_address = match DEFAULT_LSP_ADDRESS.parse() {
            Ok(addr) => addr,
            Err(e) => {
                println!("Error parsing LSP address: {:?}, using default", e);
                "127.0.0.1:9737".parse().unwrap()
            }
        };
        builder.set_liquidity_source_lsps2(
            lsp_pubkey,
            lsp_address, 
            Some(DEFAULT_LSP_AUTH.to_string())
        );
    }
    
    // Configure the network
    let network = match DEFAULT_NETWORK.to_lowercase().as_str() {
        "signet" => Network::Signet,
        "testnet" => Network::Testnet,
        "bitcoin" => Network::Bitcoin,
        _ => {
            println!("Warning: Unknown network in config, defaulting to Signet");
            Network::Signet
        }
    };
    
    println!("Setting network to: {:?}", network);
    builder.set_network(network);
    
    // Set up Esplora chain source
    println!("Setting Esplora API URL: {}", DEFAULT_CHAIN_SOURCE_URL);
    builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
    
    // Set up data directory
    println!("Setting storage directory: {}", USER_DATA_DIR);
    
    // Ensure the data directory exists
    if !std::path::Path::new(USER_DATA_DIR).exists() {
        println!("Creating data directory: {}", USER_DATA_DIR);
        std::fs::create_dir_all(USER_DATA_DIR).unwrap_or_else(|e| {
            println!("WARNING: Failed to create data directory: {}. Error: {}", USER_DATA_DIR, e);
        });
    }
    
    builder.set_storage_dir_path(USER_DATA_DIR.to_string());
    
    // Set up listening address for the user node
    let listen_addr = format!("127.0.0.1:{}", USER_PORT).parse().unwrap();
    println!("Setting listening address: {}", listen_addr);
    builder.set_listening_addresses(vec![listen_addr]).unwrap();
    
    // Set node alias
    builder.set_node_alias(USER_NODE_ALIAS.to_string());
    
    // Build the node
    let node = match builder.build() {
        Ok(node) => {
            println!("User node built successfully");
            node
        },
        Err(e) => {
            panic!("Failed to build user node: {:?}", e);
        }
    };
    
    // Start the node
    if let Err(e) = node.start() {
        panic!("Failed to start user node: {:?}", e);
    }
    
    println!("User node started with ID: {}", node.node_id());
    println!("To connect to this node, use:");
    println!("  openchannel {} 127.0.0.1:{} [SATS_AMOUNT]", node.node_id(), USER_PORT);
    
    node
}

// Enum to track the application state
enum UIState {
    OnboardingScreen,
    WaitingForPayment,
    MainScreen,
    ClosingScreen
}

// Main application structure for GUI
pub struct StableChannelsApp {
    state: UIState,
    last_stability_check: Instant,
    invoice_result: String,
    state_manager: StateManager,
    qr_texture: Option<TextureHandle>,
    status_message: String,
    close_channel_address: String,
}

impl StableChannelsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Ensure user directory exists
        if !std::path::Path::new(USER_DATA_DIR).exists() {
            std::fs::create_dir_all(USER_DATA_DIR).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to create directories: {}", e);
            });
        }
    
        let user = make_user_node();

        let channels = state_manager.node().list_channels();
        let state = if channels.is_empty() {
            UIState::OnboardingScreen
        } else {
            UIState::MainScreen
        }; 

        Self {
            state,
            last_stability_check: Instant::now() - Duration::from_secs(60),
            invoice_result: String::new(),
            state_manager,
            qr_texture: None,
            status_message: String::new(),
            close_channel_address: String::new(),
        }
    }

    fn check_stability(&mut self) {
        let action = self.state_manager.check_stability();
        
        match action {
            StabilityAction::DoNothing => {
                self.status_message = "Difference from par less than 0.1%. Stable.".to_string();
            },
            StabilityAction::Wait => {
                self.status_message = "Waiting for payment from counterparty...".to_string();
            },
            StabilityAction::Pay(amt) => {
                self.status_message = "Paying the difference...".to_string();
                
                match self.state_manager.execute_payment(amt) {
                    Ok(payment_id) => {
                        self.status_message = format!("Payment sent successfully with ID: {}", payment_id);
                        println!("Payment sent successfully with payment ID: {}", payment_id);
                    },
                    Err(e) => {
                        self.status_message = format!("Failed to send payment: {}", e);
                        println!("Failed to send payment: {}", e);
                    }
                }
            },
            StabilityAction::HighRisk(risk_level) => {
                self.status_message = format!("Risk level high: {}", risk_level);
            },

            StabilityAction::NotInitialized => {
                self.status_message = "Channel not properly initialized. Please create a channel first.".to_string();
                println!("Channel not properly initialized. Please create a channel first.");
            }
        }
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {    
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new("Stable Channel JIT payment".to_string()).unwrap()
        );
        
        // Use the default expected USD amount
        let amount_msats = (DEFAULT_EXPECTED_USD * 1_000_000.0) as u64;

        let result = user.bolt11_payment().receive_via_jit_channel(
            amount_msats,
            &description,
            3600,
            Some(10_000_000),
        );
    
        match result {
            Ok(invoice) => {
                self.invoice_result = invoice.to_string();
                let code = QrCode::new(&self.invoice_result).unwrap_or_else(|_| QrCode::new("Error generating QR").unwrap());
                let bits = code.to_colors();
                let width = code.width();
                let scale_factor = 4;
                let mut imgbuf =
                    GrayImage::new((width * scale_factor) as u32, (width * scale_factor) as u32);
    
                for y in 0..width {
                    for x in 0..width {
                        let color = if bits[y * width + x] == Color::Dark {
                            0
                        } else {
                            255
                        };
                        for dy in 0..scale_factor {
                            for dx in 0..scale_factor {
                                imgbuf.put_pixel(
                                    (x * scale_factor + dx) as u32,
                                    (y * scale_factor + dy) as u32,
                                    Luma([color]),
                                );
                            }
                        }
                    }
                }
                let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for pixel in imgbuf.pixels() {
                    let lum = pixel[0];
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(255);
                }
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.qr_texture =
                    Some(ctx.load_texture("qr_code", color_image, TextureOptions::LINEAR));
            }
            Err(e) => {
                self.invoice_result = format!("Error: {e:?}");
            }
        }
    }

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Stable Channels v0.2")
                        .size(28.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(50.0);
    
                // Step 1
                ui.heading(
                    egui::RichText::new("Step 1: Get a Lightning invoice ⚡")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(r#"Press the "Stabilize" button below."#)
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 2
                ui.heading(
                    egui::RichText::new("Step 2: Send yourself bitcoin 💸")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Over Lightning, from an app or an exchange.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 3
                ui.heading(
                    egui::RichText::new("Step 3: Stable channel created 🔧")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Self-custody. Your keys, your coins.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(50.0);
    
                // Create channel button
                let subtle_orange = egui::Color32::from_rgba_premultiplied(247, 147, 26, 200); 
                let create_channel_button = egui::Button::new(
                    egui::RichText::new("Stabilize")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0),
                )
                .min_size(egui::vec2(200.0, 55.0))
                .fill(subtle_orange)
                .rounding(8.0);
    
                if ui.add(create_channel_button).clicked() {
                    self.get_jit_invoice(ctx);
                    self.state = UIState::WaitingForPayment;
                }
            });
        });
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);

            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Send yourself bitcoin to stabilize.")
                        .size(16.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(3.0);
                ui.label("This is a Bolt11 Lightning invoice.");
                ui.add_space(8.0);

                if let Some(ref qr) = self.qr_texture {
                    ui.image(qr);
                } else {
                    ui.label("Lightning QR Missing");
                }

                ui.add_space(8.0);

                ui.add(
                    egui::TextEdit::multiline(&mut self.invoice_result)
                        .frame(true)
                        .desired_width(400.0)
                        .desired_rows(3)
                        .hint_text("Invoice..."),
                );

                ui.add_space(8.0);

                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Copy Invoice")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0),
                ).clicked() {
                    ctx.output_mut(|o| {
                        o.copied_text = self.invoice_result.clone();
                    });
                }
                
                ui.add_space(5.0); 
                
                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Back")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0), 
                ).clicked() {
                    self.state = UIState::OnboardingScreen;
                }
                
                ui.add_space(8.0); 
            });
        });
    }

    fn show_main_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::none()
                .inner_margin(Margin::symmetric(20.0, 0.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        // --- Existing Balance UI ---
                        let balances = user.list_balances();
                        let sc = self.state_manager.get_stable_channel();
                        let lightning_balance_btc = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                        let lightning_balance_usd = USD::from_bitcoin(lightning_balance_btc, sc.latest_price);
              
                        ui.add_space(30.0);

                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.heading("Your Stable Balance");
                            ui.add(egui::Label::new(
                                egui::RichText::new(lightning_balance_usd.to_string())
                                    .size(36.0)
                                    .strong(),
                            ));
                            ui.label(format!("Agreed Peg USD: {}", sc.expected_usd));
                            ui.label(format!("Bitcoin: {}", lightning_balance_btc.to_string()));
                            ui.add_space(20.0);
                        });

                        ui.add_space(20.0);

                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.heading("Bitcoin Price");
                            ui.label(format!("${:.2}", sc.latest_price));
                            ui.add_space(20.0);

                            let last_updated = self.last_stability_check.elapsed().as_secs();
                            ui.add_space(5.0);
                            ui.label(
                                egui::RichText::new(format!("Last updated: {}s ago", last_updated))
                                    .size(12.0)
                                    .color(egui::Color32::GRAY),
                            );
                        });

                        ui.add_space(20.0);

                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                ui.collapsing("Close Channel", |ui| {
                                    ui.label("Withdrawal address (minus transaction fees):");
                                    ui.add_space(10.0);
                                    ui.text_edit_singleline(&mut self.close_channel_address);
                                    ui.add_space(10.0);

                                    if ui.add(
                                        egui::Button::new(
                                            egui::RichText::new("Close Channel")
                                                .color(egui::Color32::WHITE)
                                                .size(12.0),
                                        )
                                        .rounding(6.0),
                                    )
                                    .clicked()
                                    {
                                        self.close_all_channels_to_address();
                                    }
                                });

                                ui.add_space(20.0);

                                if !self.status_message.is_empty() {
                                    ui.label(self.status_message.clone());
                                }
                            });
                    });
                });
        });
    }

    fn show_closing_screen(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.heading(
                    egui::RichText::new(format!("Withdrawal processing")).size(28.0).strong(),
                );    
            });
    
            ui.add_space(20.0);
            ui.horizontal_centered(|ui| {
                ui.heading(                    
                    egui::RichText::new(format!("{}",self.close_channel_address)).size(28.0).strong(), 
                );
            });
        });
    }

    fn poll_for_events(&mut self) {
        while let Some(event) = user.next_event() {
            match event {
                Event::ChannelReady { .. } => {
                    // Once we have a channel, initialize the stable channel
                    let channels = user.list_channels();
                    if let Some(channel) = channels.first() {
                        // Use the first channel we find
                        if let Err(e) = self.state_manager.initialize_stable_channel(
                            &channel.channel_id.to_string(),
                            true, // default to stable receiver
                            DEFAULT_EXPECTED_USD,
                            0.0, // no native bitcoin amount
                        ) {
                            eprintln!("Error initializing stable channel: {:?}", e);
                        }
                    }
                    self.check_stability();
                    self.state = UIState::MainScreen;
                }
                
                Event::PaymentReceived { .. } => {
                    self.state = UIState::MainScreen;
                    println!("Payment received");
                }

                Event::ChannelClosed { .. } => {
                    if user.list_channels().is_empty() {
                        println!("All channels closed, returning to onboarding screen");
                        self.state = UIState::OnboardingScreen;
                    } else {
                        self.state = UIState::ClosingScreen;
                        println!("Channel closed, but other channels still exist");
                    }
                }
                _ => {}
            }
            user.event_handled();
        }
    }

    fn close_all_channels_to_address(&mut self) {
        if self.close_channel_address.is_empty() {
            self.status_message = "Please enter a withdrawal address".to_string();
            return;
        }

        for channel in user.list_channels().iter() {
            let user_channel_id = channel.user_channel_id.clone();
            let counterparty_node_id = channel.counterparty_node_id;
            match user.close_channel(&user_channel_id, counterparty_node_id) {
                Ok(_) => self.status_message = "Closing channel...".to_string(),
                Err(e) => self.status_message = format!("Error closing channel: {}", e),
            }
        }

        // Withdraw everything to address
        match ldk_node::bitcoin::Address::from_str(&self.close_channel_address) {
            Ok(addr) => {
                let network = Network::Signet;
                
                match addr.require_network(network) {
                    Ok(addr_checked) => {
                        match user.onchain_payment().send_all_to_address(&addr_checked, false, None) {
                            Ok(txid) => {
                                self.status_message = format!("Withdrawal transaction sent: {}", txid);
                                self.state = UIState::ClosingScreen;
                            },
                            Err(e) => self.status_message = format!("Error sending withdrawal: {}", e),
                        }
                    },
                    Err(_) => self.status_message = "Invalid address for this network".to_string(),
                }
            },
            Err(_) => self.status_message = "Invalid address format".to_string(),
        }
    }
}

impl eframe::App for StableChannelsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        let now = Instant::now();
        
        if now.duration_since(self.last_stability_check) >= Duration::from_secs(30) {
            self.check_stability();
            self.last_stability_check = now;
        }

        match self.state {
            UIState::OnboardingScreen => self.show_onboarding_screen(ctx),
            UIState::WaitingForPayment => self.show_waiting_for_payment_screen(ctx),
            UIState::MainScreen => self.show_main_screen(ctx),
            UIState::ClosingScreen => self.show_closing_screen(ctx),
        }

        self.poll_for_events();
    }
}

// Command-line interface implementation
fn run_cli(user: Node) {
    println!("\n=== Stable Channels User Interface ===");
    println!("Type 'help' for available commands");
    
    loop {
        // Process any pending events
        while let Some(event) = user.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    println!("Channel {} is now ready", channel_id);
                    
                    // Try to initialize the stable channel if none was initialized before
                    if !user.is_initialized() {
                        let channels = user.list_channels();
                        if let Some(channel) = channels.first() {
                            // Use the first channel we find
                            if let Err(e) = user.initialize_stable_channel(
                                &channel.channel_id.to_string(),
                                true, // default to stable receiver
                                20.0, // Default expected USD amount
                                0.0,  // no native bitcoin amount
                            ) {
                                eprintln!("Error initializing stable channel: {:?}", e);
                            } else {
                                println!("Automatically initialized stable channel with ID: {}", channel.channel_id);
                            }
                        }
                    }
                },
                Event::PaymentReceived { payment_hash, amount_msat, .. } => {
                    println!("Received payment: {} msat (hash: {})", amount_msat, payment_hash);
                },
                Event::ChannelClosed { channel_id, .. } => {
                    println!("Channel {} has been closed", channel_id);
                },
                _ => {}
            }
            user.event_handled();
        }
        
        let (_input, command, args) = get_user_input("Enter command for user: ");

        match (command.as_deref(), args.as_slice()) {
            (Some("help"), []) => {
                println!("\nAvailable commands:");
                println!("  getaddress                        - Get a new Bitcoin address for deposits");
                println!("  balance                           - Show current balances");
                println!("  checkstability                    - Check stability and take action if needed");
                println!("  listallchannels                   - List all available channels");
                println!("  openchannel <node_id> <address> <sats> - Open a new channel");
                println!("  startstablechannel <channel_id> <is_receiver> <usd_amount> [<btc_amount>] - Initialize stable channel");
                println!("  getinvoice <sats>                 - Create a payment invoice");
                println!("  getjitinvoice                     - Create a JIT channel payment invoice");
                println!("  payinvoice <bolt11>               - Pay a BOLT11 invoice");
                println!("  closeallchannels                  - Close all channels");
                println!("  exit                              - Exit the application");
            },
            (Some("settheiroffer"), [their_offer_str]) => {
                match Offer::from_str(their_offer_str) {
                    Ok(_offer) => {
                        println!("Offer set.")
                    },
                    Err(_) => println!("Error parsing offer"),
                }
            }
            (Some("getouroffer"), []) => {
                match user.bolt12_payment().receive_variable_amount("thanks", None) {
                    Ok(our_offer) => println!("{}", our_offer),
                    Err(e) => println!("Error creating offer: {}", e),
                }
            }
            (Some("checkstability"), []) => {
                let action = user.check_stability();
                match action {
                    StabilityAction::Pay(amount) => {
                        println!("Action: Pay {} msats", amount);
                        match user.execute_payment(amount) {
                            Ok(payment_id) => println!("Payment sent with ID: {}", payment_id),
                            Err(e) => println!("Failed to send payment: {}", e),
                        }
                    },
                    StabilityAction::Wait => println!("Action: Wait for counterparty payment"),
                    StabilityAction::DoNothing => println!("Action: Do nothing, channel is stable"),
                    StabilityAction::HighRisk(risk) => println!("Action: High risk level ({})", risk),
                    StabilityAction::NotInitialized => {
                        println!("Channel not properly initialized or may have been closed.");
                    }
                }
            }
            (Some("startstablechannel"), [channel_id, is_stable_receiver, expected_dollar_amount, native_amount_sats]) => {
                let channel_id = channel_id.to_string();
                let is_stable_receiver = match is_stable_receiver.parse::<bool>() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("Error: is_stable_receiver must be 'true' or 'false'");
                        continue;
                    }
                };
                
                let expected_dollar_amount = match expected_dollar_amount.parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("Error: expected_dollar_amount must be a valid number");
                        continue;
                    }
                };
                
                let native_amount_sats = match native_amount_sats.parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("Error: native_amount_sats must be a valid number");
                        continue;
                    }
                };

                match user.initialize_stable_channel(
                    &channel_id, 
                    is_stable_receiver, 
                    expected_dollar_amount, 
                    native_amount_sats
                ) {
                    Ok(()) => {
                        println!("Stable Channel initialized: {}", channel_id);
                    },
                    Err(e) => println!("Failed to initialize stable channel: {}", e),
                }
            }
            (Some("getaddress"), []) => {
                let funding_address = user.onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("User Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            }
            (Some("openchannel"), args) => {
                if args.len() != 3 {
                    println!("Error: 'openchannel' command requires three parameters: <node_id>, <listening_address>, and <sats>");
                    continue;
                }

                let node_id_str = &args[0];
                let listening_address_str = &args[1];
                let sats_str = &args[2];

                let lsp_node_id = match node_id_str.parse() {
                    Ok(id) => id,
                    Err(e) => {
                        println!("Failed to parse node ID: {}", e);
                        continue;
                    }
                };
                
                let lsp_net_address: SocketAddress = match listening_address_str.parse() {
                    Ok(addr) => addr,
                    Err(e) => {
                        println!("Failed to parse address: {}", e);
                        continue;
                    }
                };
                
                let sats: u64 = match sats_str.parse() {
                    Ok(s) => s,
                    Err(e) => {
                        println!("Failed to parse sats amount: {}", e);
                        continue;
                    }
                };
                
                let push_msat = (sats / 2) * 1000;
                let channel_config: Option<ChannelConfig> = None;

                match user.open_announced_channel(
                    lsp_node_id,
                    lsp_net_address,
                    sats,
                    Some(push_msat),
                    channel_config,
                ) {
                    Ok(_) => println!("Channel successfully opened to {}", node_id_str),
                    Err(e) => println!("Failed to open channel: {}", e),
                }
            }
            (Some("balance"), []) => {
                let balances = user.list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance =
                    Bitcoin::from_sats(balances.total_lightning_balance_sats);
                
                // Get stable channel info if initialized
                let sc = user.get_stable_channel();
                if sc.latest_price > 0.0 {
                    println!("User On-Chain Balance: {}", onchain_balance);
                    println!("User Lightning Balance: {}", lightning_balance);
                    println!("Current BTC/USD Price: ${:.2}", sc.latest_price);
                    
                    // Print stable channel balances
                    if sc.is_stable_receiver {
                        // User is the receiver
                        println!("User Receiver Balance: {} (${:.2})",
                            sc.stable_receiver_btc,
                            sc.stable_receiver_usd.0);
                        println!("LSP Provider Balance: {} (${:.2})",
                            sc.stable_provider_btc,
                            sc.stable_provider_usd.0);
                    } else {
                        // User is the provider
                        println!("User Provider Balance: {} (${:.2})",
                            sc.stable_provider_btc,
                            sc.stable_provider_usd.0);
                        println!("LSP Receiver Balance: {} (${:.2})",
                            sc.stable_receiver_btc,
                            sc.stable_receiver_usd.0);
                    }
                } else {
                    println!("User On-Chain Balance: {}", onchain_balance);
                    println!("User Lightning Balance: {}", lightning_balance);
                }
            }
            (Some("closeallchannels"), []) => {
                for channel in user.list_channels().iter() {
                    let mut user_channel_id = channel.user_channel_id;
                    let counterparty_node_id = channel.counterparty_node_id;
                    let _ = user.close_channel(&user_channel_id, counterparty_node_id);
                }
                println!("Closing all channels.")
            }
            (Some("listallchannels"), []) => {
                let channels = user.list_channels();
                if channels.is_empty() {
                    println!("No channels found.");
                } else {
                    println!("User Channels:");
                    for channel in channels.iter() {
                        println!("--------------------------------------------");
                        println!("Channel ID: {}", channel.channel_id);
                        println!(
                            "Channel Value: {}",
                            Bitcoin::from_sats(channel.channel_value_sats)
                        );
                        println!("Channel Ready?: {}", channel.is_channel_ready);
                    }
                    println!("--------------------------------------------");
                }
            }
            (Some("getinvoice"), [sats]) => {
                if let Ok(sats_value) = sats.parse::<u64>() {
                    let msats = sats_value * 1000;
                    let bolt11 = user.bolt11_payment();
                    let description = Bolt11InvoiceDescription::Direct(
                        Description::new("Invoice".to_string()).unwrap_or_else(|_| {
                            println!("Failed to create description, using fallback");
                            Description::new("Fallback Invoice".to_string()).unwrap()
                        })
                    );
                    
                    match bolt11.receive(msats, &description, 6000) {
                        Ok(inv) => println!("User Invoice: {}", inv),
                        Err(e) => println!("Error creating invoice: {}", e),
                    }
                } else {
                    println!("Invalid sats value provided");
                }
            }
            (Some("payinvoice"), [invoice_str]) => {
                let bolt11_invoice = match invoice_str.parse::<Bolt11Invoice>() {
                    Ok(invoice) => invoice,
                    Err(e) => {
                        println!("Error parsing invoice: {}", e);
                        continue;
                    }
                };
                
                match user.bolt11_payment().send(&bolt11_invoice, None) {
                    Ok(payment_id) => {
                        println!("Payment sent from User with payment_id: {}", payment_id)
                    }
                    Err(e) => println!("Error sending payment from User: {}", e),
                }
            }
            (Some("getjitinvoice"), []) => {
                let description = Bolt11InvoiceDescription::Direct(
                    Description::new("Stable Channel JIT payment".to_string()).unwrap_or_else(|_| {
                        println!("Failed to create description, using fallback");
                        Description::new("Fallback JIT Invoice".to_string()).unwrap()
                    })
                );
                
                match user.bolt11_payment().receive_via_jit_channel(
                    50000000,
                    &description,
                    3600,
                    Some(10000000),
                ) {
                    Ok(invoice) => println!("Invoice: {}", invoice.to_string()),
                    Err(e) => println!("Error: {}", e),
                }
            }
            (Some("exit"), _) => break,
            (None, _) => {}, // Empty input, just loop
            _ => println!("Unknown command or incorrect arguments. Type 'help' for available commands."),
        }
    }
}

// Main function to launch the app
pub fn run() {    
    // Default to starting the graphical app
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([460.0, 700.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "Stable Channels",
        native_options,
        Box::new(|cc| {
            Ok(Box::new(StableChannelsApp::new(cc)))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error running application: {:?}", e);
        
        // If GUI fails to start, fall back to CLI
        println!("GUI could not be started, falling back to CLI mode");
        
        // Parse LSP pubkey
        let lsp_pubkey_bytes = match hex::decode(DEFAULT_LSP_PUBKEY) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("Error decoding LSP pubkey: {:?}", e);
                vec![0; 33] // Fallback to empty pubkey
            }
        };

        let lsp_pubkey = match PublicKey::from_slice(&lsp_pubkey_bytes) {
            Ok(key) => Some(key),
            Err(e) => {
                eprintln!("Error parsing LSP pubkey: {:?}", e);
                None
            }
        };

        let is_service = false; 
        let user = make_user_node();
        
        // run_cli(user);
    });
}