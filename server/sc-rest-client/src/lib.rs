//! Client-side library to interact with the Stable Channels LSP daemon over REST.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]
#![deny(missing_docs)]

/// Implements a [`client::LspRestClient`] (REST client) to access a Stable Channels daemon.
pub mod client;

/// Implements the error type ([`error::LspRestError`]) returned on interacting with [`client::LspRestClient`].
pub mod error;

/// Stable Channels-specific REST proto types (GetPrice, ListStableChannels, etc.).
pub use sc_protos;

/// LDK Server gRPC proto types used for proxied endpoints.
pub use ldk_server_client::ldk_server_grpc;
