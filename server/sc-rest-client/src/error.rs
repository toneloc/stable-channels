use std::fmt;

/// Represents an error returned by the SC daemon (or its REST client).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LspRestError {
	/// The error message containing a generic description of the error condition in English.
	/// It is intended for a human audience only and should not be parsed to extract any information
	/// programmatically. Client-side code may use it for logging only.
	pub message: String,

	/// The error code uniquely identifying an error condition.
	/// It is meant to be read and understood programmatically by code that detects/handles errors by
	/// type.
	pub error_code: LspRestErrorCode,
}

impl LspRestError {
	/// Creates a new [`LspRestError`] with the given error code and message.
	pub fn new(error_code: LspRestErrorCode, message: impl Into<String>) -> Self {
		Self { error_code, message: message.into() }
	}
}

impl std::error::Error for LspRestError {}

impl fmt::Display for LspRestError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Error: [{}]: {}", self.error_code, self.message)
	}
}

/// Defines error codes for categorizing SC daemon REST errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LspRestErrorCode {
	/// Please refer to [`ldk_server_grpc::error::ErrorCode::InvalidRequestError`].
	InvalidRequestError,

	/// Please refer to [`ldk_server_grpc::error::ErrorCode::AuthError`].
	AuthError,

	/// Please refer to [`ldk_server_grpc::error::ErrorCode::LightningError`].
	LightningError,

	/// Please refer to [`ldk_server_grpc::error::ErrorCode::InternalServerError`].
	InternalServerError,

	/// There is an unknown error, it could be a client-side bug, unrecognized error-code, network error
	/// or something else.
	InternalError,
}

impl fmt::Display for LspRestErrorCode {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			LspRestErrorCode::InvalidRequestError => write!(f, "InvalidRequestError"),
			LspRestErrorCode::AuthError => write!(f, "AuthError"),
			LspRestErrorCode::LightningError => write!(f, "LightningError"),
			LspRestErrorCode::InternalServerError => write!(f, "InternalServerError"),
			LspRestErrorCode::InternalError => write!(f, "InternalError"),
		}
	}
}
