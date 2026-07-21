// On Windows release builds, ship as a GUI app: no console window pops up
// alongside the eframe window. Debug builds keep the console so panics and
// `eprintln!` from `cargo run` remain visible during dev.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

/// Stable Channels in LDK
/// Contents
/// Main data structure and helper types are in `types.rs`.
/// The price feed config and logic is in price_feeds.rs.
/// User-facing (stability) code in user.rs
/// Server code in server.rs
/// This present file includes LDK set-up, program initialization,
/// a command-line interface, and the core stability logic.
/// We have three different services: exchange, user, and lsp
use std::env;
use regex::Regex;
use tracing_subscriber::{
    fmt::{self, format::Writer},
    EnvFilter,
    layer::SubscriberExt,
};
use tracing_appender::rolling;
use std::fmt::Result as FmtResult;

pub mod audit;
pub mod constants;
pub mod price_feeds;
pub mod stable;
pub mod types;
pub mod user;

fn main() {
    // Desktop Logging setup
    let data_dir = constants::get_user_data_dir();
    let file_appender = rolling::never(data_dir, "app_debug.log");
    
    // Add custom seed redaction logic before writing to file
    struct RedactingFormatter;
    
    impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for RedactingFormatter
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
        N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
    {
        fn format_event(
            &self,
            ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
            mut writer: Writer<'_>,
            event: &tracing::Event<'_>,
        ) -> FmtResult {
            let mut buf = String::new();
            let mut visitor = StringVisitor(&mut buf);
            event.record(&mut visitor);
            
            // Simple regex to match 12-24 word patterns
            let re = Regex::new(r"(?:\b[a-z]+\s+){11,23}[a-z]+\b").unwrap();
            let redacted = re.replace_all(&buf, "[REDACTED_SEED]");
            
            let meta = event.metadata();
            write!(
                writer,
                "{} [{}] [{}] {}\n",
                chrono::Utc::now().to_rfc3339(),
                meta.level(),
                meta.target(),
                redacted
            )
        }
    }

    struct StringVisitor<'a>(&'a mut String);
    impl<'a> tracing::field::Visit for StringVisitor<'a> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.0, "{}={:?} ", field.name(), value);
        }
    }

    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    let file_layer = fmt::layer()
        .event_format(RedactingFormatter)
        .with_writer(non_blocking);

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(file_layer);

    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("Failed to set global logger: {}", e);
    }
    
    tracing::info!("Logger initialized");

    let mode = env::args().nth(1).unwrap_or_else(|| "user".to_string());

    match mode.as_str() {
        "user" => user::run(),
        // "lsp" | "exchange" => server::run_with_mode(&mode),
        _ => {
            eprintln!(
                "Unknown mode: '{}'. Use: `user`, `lsp`, or `exchange`",
                mode
            );
            std::process::exit(1);
        }
    }
}
