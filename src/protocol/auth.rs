//! Authentication types for OAuth and JWT.
//!
//! This module contains types for handling authentication responses from Deezer's
//! login endpoints, including:
//!
//! * OAuth authentication responses
//!   - Access tokens for API access
//!   - Token expiration information
//!
//! * JWT authentication
//!   - ARL (Advanced Request Log) tokens
//!   - Account identification
//!
//! # Example OAuth Response
//!
//! ```json
//! {
//!     "access_token": "secret_token",
//!     "expire": 0,
//!     "expires": 0
//! }
//! ```
//!
//! # Example JWT Payload
//!
//! ```json
//! {
//!     "arl": "secret_arl_token",
//!     "account_id": "12345"
//! }
//! ```
//!
//! # Authentication Flow
//!
//! 1. Initial login provides OAuth access token
//! 2. Access token used to obtain JWT with ARL
//! 3. ARL stored as cookie for persistent authentication
//! 4. JWT renewed automatically before expiration
//!
//! # Note
//!
//! While the OAuth response includes expiration fields, Deezer currently returns 0
//! for both `expire` and `expires`. Token expiration is handled through JWT renewal
//! and cookie management instead.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_with::{DurationSeconds, TimestampSeconds, formats::Flexible, serde_as};
use veil::Redact;

/// User authentication data from OAuth login.
///
/// Contains the access token and timing information needed
/// to maintain an authenticated session.
#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Redact, Hash)]
pub struct User {
    /// OAuth access token for API authentication
    #[redact]
    pub access_token: String,

    /// How long the token remains valid
    ///
    /// Note: Currently always returns 0 in Deezer responses
    #[serde_as(as = "Option<DurationSeconds<u64, Flexible>>")]
    pub expire: Option<Duration>,

    /// When the token will expire
    ///
    /// Note: Currently always returns 0 in Deezer responses
    #[serde_as(as = "Option<TimestampSeconds<i64, Flexible>>")]
    pub expires: Option<SystemTime>,
}

/// JWT payload for persistent authentication.
///
/// Contains the tokens and identifiers needed to maintain a persistent
/// authenticated session across client restarts.
///
/// The ARL token is stored as a cookie and automatically renewed
/// before expiration to maintain the session.
#[derive(Clone, Debug, Serialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Jwt {
    /// Authentication Reference Links for persistent authentication
    pub arl: String,

    /// Unique identifier for the authenticated account
    pub account_id: String,
}
