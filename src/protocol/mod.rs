//! Protocol types and structures for Deezer services.
//!
//! This module contains the core data types and parsing logic used across
//! different Deezer protocols and APIs:
//!
//! # Submodules
//!
//! * [`auth`] - OAuth authentication response types
//! * [`connect`] - Deezer Connect protocol for remote playback control
//! * [`gateway`] - Gateway API for user data and authentication
//! * [`media`] - Media streaming and track access
//!
//! # Shared Functionality
//!
//! The module provides common utilities for protocol handling:
//!
//! * JSON parsing with consistent error handling
//! * Structured logging of API responses
//! * Debug output for protocol analysis
//!
//! # Usage Example
//!
//! ```
//! use pleezer::protocol;
//!
//! // Parse and log JSON response
//! let response: MyType = protocol::json(&body, "endpoint_name")?;
//!
//! // Response is logged at:
//! // - TRACE level if successful
//! // - ERROR level with details if parsing fails
//! ```
//!
//! # Module Organization
//!
//! Each submodule handles a specific part of the Deezer protocol:
//!
//! * `auth` - Initial OAuth authentication
//! * `connect` - Remote playback and control
//! * `gateway` - User session and data access
//! * `media` - Track streaming and downloads
//!
//! Modules are designed to work together while maintaining separation
//! of concerns between different protocol aspects.

pub mod auth;
pub mod codec;
pub mod connect;
pub mod gateway;
pub mod media;

pub use codec::Codec;

use crate::error::Result;
use serde::Deserialize;
use std::fmt::Debug;

/// Parses and logs JSON responses from Deezer APIs.
///
/// # Arguments
///
/// * `body` - Response body text to parse
/// * `origin` - Description of API endpoint for logging
///
/// # Type Parameters
///
/// * `T` - Response type that implements `Deserialize` and `Debug`
///
/// # Returns
///
/// * `Ok(T)` - Successfully parsed response
/// * `Err` - Parse error with debug logging of response
///
/// # Errors
///
/// Returns error if:
/// * Response body is not valid JSON
/// * JSON structure doesn't match type `T`
/// * Deserialization fails for any field
///
/// # Logging
///
/// * Success: Logs parsed structure at TRACE level
/// * Parse Error: Logs raw JSON at TRACE level if valid JSON
/// * Invalid JSON: Logs error and raw text at ERROR level
pub fn json<T>(body: &str, origin: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Debug,
{
    match serde_json::from_str(body) {
        Ok(result) => {
            trace!("{origin}: {result:#?}");
            Ok(result)
        }
        Err(e) => {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
                trace!("{origin}: {json:#?}");
            } else {
                error!("{origin}: failed parsing response ({e:?})");
                trace!("{body}");
            }
            Err(e.into())
        }
    }
}
