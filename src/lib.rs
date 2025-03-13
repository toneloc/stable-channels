// Export modules for external use
pub mod types;
pub mod price_feeds;
pub mod state;

// Re-export key structures
pub use state::StateManager;
pub use state::StabilityAction;
pub use types::{Bitcoin, USD, StableChannel};