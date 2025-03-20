//! Stream reporting messages in the Deezer Connect protocol.
//!
//! This module handles messages that report active playback streams between
//! devices. These reports enable features like:
//! * Playback limitation enforcement
//! * Usage tracking and monetization
//! * Cross-device playback coordination
//!
//! Stream reports identify:
//! * The playing user ([`UserId`])
//! * The active track ([`TrackId`])
//! * The unique stream session ([`Uuid`])
//!
//! # Wire Format
//!
//! Stream messages use a specific JSON format:
//! ```json
//! {
//!     "ACTION": "PLAY",
//!     "APP": "LIMITATION",
//!     "VALUE": {
//!         "USER_ID": "123456789",
//!         "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
//!         "SNG_ID": "987654321"
//!     }
//! }
//! ```
//!
//! # Examples
//!
//! ```rust
//! use uuid::Uuid;
//! use deezer::stream::{Action, Contents, Ident, Value};
//!
//! let contents = Contents {
//!     action: Action::Play,
//!     ident: Ident::Limitation,
//!     value: Value {
//!         user: 123456789.into(),
//!         uuid: Uuid::new_v4(),
//!         track_id: 987654321.into(),
//!     },
//! };
//! ```

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, DisplayFromStr, SerializeDisplay, serde_as};
use uuid::Uuid;

use super::channel::UserId;
use crate::{error::Error, track::TrackId};

// Contents of a stream report message.
///
/// Stream reports inform other devices about active playback streams, including
/// who is playing what track. These reports are used to manage concurrent
/// playback limitations.
///
/// # Wire Format
///
/// ```json
/// {
///     "ACTION": "PLAY",
///     "APP": "LIMITATION",
///     "VALUE": {
///         "USER_ID": "123456789",
///         "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
///         "SNG_ID": "987654321"
///     }
/// }
/// ```
///
/// # Validation Rules
///
/// The following rules are enforced during serialization/deserialization:
/// * `USER_ID` must be a valid positive integer or "-1"
/// * `UNIQID` must be a valid UUID string
/// * `SNG_ID` must be a valid track ID (positive or negative integer)
/// * `ACTION` must be a known action type
/// * `APP` must be "LIMITATION"
///
/// # Examples
///
/// Valid message:
/// ```rust
/// use uuid::Uuid;
/// use deezer::stream::{Action, Contents, Ident, Value};
///
/// let contents = Contents {
///     action: Action::Play,
///     ident: Ident::Limitation,
///     value: Value {
///         user: 123456789.into(),
///         uuid: Uuid::new_v4(),
///         track_id: 987654321.into(),
///     },
/// };
/// ```
///
/// Error cases:
/// ```rust
/// use serde_json::json;
///
/// // Invalid user ID
/// let invalid = json!({
///     "ACTION": "PLAY",
///     "APP": "LIMITATION",
///     "VALUE": {
///         "USER_ID": "0",  // Must be positive or -1
///         "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
///         "SNG_ID": "987654321"
///     }
/// });
/// assert!(serde_json::from_value::<Contents>(invalid).is_err());
///
/// // Unknown action
/// let invalid = json!({
///     "ACTION": "UNKNOWN",
///     "APP": "LIMITATION",
///     "VALUE": {
///         "USER_ID": "123456789",
///         "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
///         "SNG_ID": "987654321"
///     }
/// });
/// assert!(serde_json::from_value::<Contents>(invalid).is_err());
/// ```
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contents {
    /// Action being reported (e.g., starting playback).
    #[serde(rename = "ACTION")]
    pub action: Action,

    /// Type of stream report.
    #[serde(rename = "APP")]
    pub ident: Ident,

    /// Details about the active stream.
    #[serde(rename = "VALUE")]
    pub value: Value,
}

/// Details about an active playback stream.
///
/// Contains identifying information about who is playing what track,
/// used for enforcing playback limitations and tracking activity.
///
/// # Wire Format
///
/// ```json
/// {
///     "USER_ID": "123456789",
///     "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
///     "SNG_ID": "987654321"
/// }
/// ```
///
/// # Validation Rules
///
/// * `USER_ID`:
///   - Must be a valid positive integer or "-1"
///   - Zero is not allowed
///   - Must parse as u64 when positive
/// * `UNIQID`:
///   - Must be a valid UUID string
///   - Both hyphenated and non-hyphenated formats accepted
///   - Version 4 UUIDs recommended but not required
/// * `SNG_ID`:
///   - Must be a non-zero integer
///   - Can be positive (Deezer tracks) or negative (user uploads)
///   - Must parse as i64
///
/// # Examples
///
/// Valid values:
/// ```rust
/// use uuid::Uuid;
/// use deezer::stream::Value;
///
/// // Normal track
/// let value = Value {
///     user: 123456789.into(),
///     uuid: Uuid::new_v4(),
///     track_id: 987654321.into(),
/// };
///
/// // User upload track
/// let value = Value {
///     user: 123456789.into(),
///     uuid: Uuid::new_v4(),
///     track_id: (-987654321).into(),
/// };
///
/// // Broadcast stream
/// let value = Value {
///     user: UserId::Unspecified,  // -1
///     uuid: Uuid::new_v4(),
///     track_id: 987654321.into(),
/// };
/// ```
///
/// Error cases:
/// ```rust
/// use serde_json::json;
///
/// // Invalid user ID
/// let invalid = json!({
///     "USER_ID": "0",  // Must be positive or -1
///     "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
///     "SNG_ID": "987654321"
/// });
/// assert!(serde_json::from_value::<Value>(invalid).is_err());
///
/// // Invalid UUID
/// let invalid = json!({
///     "USER_ID": "123456789",
///     "UNIQID": "not-a-uuid",
///     "SNG_ID": "987654321"
/// });
/// assert!(serde_json::from_value::<Value>(invalid).is_err());
///
/// // Invalid track ID
/// let invalid = json!({
///     "USER_ID": "123456789",
///     "UNIQID": "550e8400-e29b-41d4-a716-446655440000",
///     "SNG_ID": "0"  // Must be non-zero
/// });
/// assert!(serde_json::from_value::<Value>(invalid).is_err());
/// ```
#[serde_as]
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Value {
    /// ID of the user playing the track
    #[serde(rename = "USER_ID")]
    #[serde_as(as = "DisplayFromStr")]
    pub user: UserId,

    /// Unique identifier for this stream
    #[serde(rename = "UNIQID")]
    pub uuid: Uuid,

    /// ID of the track being played
    #[serde(rename = "SNG_ID")]
    #[serde_as(as = "DisplayFromStr")]
    pub track_id: TrackId,
}

/// Action being reported in a stream message.
///
/// Identifies what type of playback activity is being reported. Currently only
/// supports reporting playback start, but the protocol structure allows for
/// future action types.
///
/// # Wire Format
///
/// Actions are serialized as uppercase strings:
/// ```json
/// {
///     "ACTION": "PLAY",
///     // ... other fields ...
/// }
/// ```
///
/// # Validation Rules
///
/// * Must be an exact string match (case sensitive)
/// * Only known actions are accepted
/// * No additional text or suffixes allowed
///
/// # Examples
///
/// Valid usage:
/// ```rust
/// use deezer::stream::Action;
///
/// // Direct construction
/// let action = Action::Play;
/// assert_eq!(action.to_string(), "PLAY");
///
/// // Parsing
/// let action: Action = "PLAY".parse()?;
/// assert_eq!(action, Action::Play);
/// ```
///
/// Error cases:
/// ```rust
/// use deezer::stream::Action;
///
/// // Case sensitivity
/// assert!("play".parse::<Action>().is_err());
///
/// // Unknown action
/// assert!("STOP".parse::<Action>().is_err());
///
/// // Invalid format
/// assert!("PLAY_TRACK".parse::<Action>().is_err());
/// ```
#[derive(Copy, Clone, Debug, SerializeDisplay, DeserializeFromStr, PartialEq, Eq, Hash)]
pub enum Action {
    /// Report that playback has started
    Play,
}

/// Type of stream report message.
///
/// Identifies the purpose of the stream report. Currently only supports
/// limitation reports (managing concurrent playback), but the protocol
/// structure allows for future report types.
///
/// # Wire Format
///
/// Identifiers are serialized as uppercase strings:
/// ```json
/// {
///     "APP": "LIMITATION",
///     // ... other fields ...
/// }
/// ```
///
/// # Validation Rules
///
/// * Must be an exact string match (case sensitive)
/// * Only known identifiers are accepted
/// * No additional text or suffixes allowed
///
/// # Examples
///
/// Valid usage:
/// ```rust
/// use deezer::stream::Ident;
///
/// // Direct construction
/// let ident = Ident::Limitation;
/// assert_eq!(ident.to_string(), "LIMITATION");
///
/// // Parsing
/// let ident: Ident = "LIMITATION".parse()?;
/// assert_eq!(ident, Ident::Limitation);
/// ```
///
/// Error cases:
/// ```rust
/// use deezer::stream::Ident;
///
/// // Case sensitivity
/// assert!("limitation".parse::<Ident>().is_err());
///
/// // Unknown identifier
/// assert!("ANALYTICS".parse::<Ident>().is_err());
///
/// // Invalid format
/// assert!("LIMITATION_V2".parse::<Ident>().is_err());
/// ```
#[derive(Copy, Clone, Debug, SerializeDisplay, DeserializeFromStr, PartialEq, Eq, Hash)]
pub enum Ident {
    /// Report related to playback limitations
    Limitation,
}

impl Action {
    /// Wire format string for the Play action
    const PLAY: &'static str = "PLAY";
}

impl Ident {
    /// Wire format string for Limitation messages
    const LIMITATION: &'static str = "LIMITATION";
}

/// Formats stream contents for display, showing action and track.
///
/// # Examples
///
/// ```rust
/// let contents = Contents {
///     action: Action::Play,
///     ident: Ident::Limitation,
///     value: Value { /* ... */ },
/// };
/// // Prints: "PLAY 987654321"
/// println!("{contents}");
/// ```
impl fmt::Display for Contents {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.action, self.value.track_id)
    }
}

/// Formats the action for wire protocol transmission.
///
/// Actions are formatted as uppercase strings:
/// * `Play` -> `"PLAY"`
///
/// # Examples
///
/// ```rust
/// assert_eq!(Action::Play.to_string(), "PLAY");
/// ```
impl fmt::Display for Action {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Play => write!(f, "{}", Self::PLAY),
        }
    }
}

/// Parses an action from its wire format string.
///
/// # Examples
///
/// ```rust
/// let action: Action = "PLAY".parse()?;
/// assert_eq!(action, Action::Play);
///
/// // Unknown actions return an error
/// assert!("UNKNOWN".parse::<Action>().is_err());
/// ```
///
/// # Errors
///
/// Returns an error if the string doesn't match a known action.
impl FromStr for Action {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let variant = match s {
            Self::PLAY => Self::Play,
            _ => return Err(Self::Err::unimplemented(format!("stream action `{s}`"))),
        };

        Ok(variant)
    }
}

/// Formats the identifier for wire protocol transmission.
///
/// Identifiers are formatted as uppercase strings:
/// * `Limitation` -> `"LIMITATION"`
///
/// # Examples
///
/// ```rust
/// assert_eq!(Ident::Limitation.to_string(), "LIMITATION");
/// ```
impl fmt::Display for Ident {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limitation => write!(f, "{}", Self::LIMITATION),
        }
    }
}

/// Parses an identifier from its wire format string.
///
/// # Examples
///
/// ```rust
/// let ident: Ident = "LIMITATION".parse()?;
/// assert_eq!(ident, Ident::Limitation);
///
/// // Unknown identifiers return an error
/// assert!("UNKNOWN".parse::<Ident>().is_err());
/// ```
///
/// # Errors
///
/// Returns an error if the string doesn't match a known identifier.
impl FromStr for Ident {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let variant = match s {
            Self::LIMITATION => Self::Limitation,
            _ => return Err(Self::Err::unimplemented(format!("stream action `{s}`"))),
        };

        Ok(variant)
    }
}
