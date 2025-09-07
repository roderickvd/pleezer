//! Message contents and data types for the Deezer Connect protocol.
//!
//! This module defines the structures and types that represent the content
//! of messages exchanged over a Deezer Connect websocket connection. It handles:
//! * Message bodies and their wire format serialization
//! * Protocol-specific data types (audio quality, repeat modes, etc.)
//! * Device identification and addressing
//! * Playback status and control information
//!
//! # Wire Format
//!
//! Messages in the Deezer Connect protocol use a specific wire format:
//! * JSON-based message envelope
//! * Base64-encoded payloads
//! * Protocol buffer messages for queue data
//! * DEFLATE compression for some payloads
//!
//! # Examples
//!
//! Creating message contents:
//! ```rust
//! use std::time::Duration;
//!
//! let contents = Contents {
//!     ident: Ident::RemoteCommand,
//!     headers: Headers {
//!         from: DeviceId::default(),
//!         destination: None,
//!     },
//!     body: Body::PlaybackProgress {
//!         message_id: "msg123".to_string(),
//!         track: QueueItem { /* ... */ },
//!         quality: AudioQuality::Standard,
//!         duration: Duration::from_secs(180),
//!         buffered: Duration::from_secs(10),
//!         progress: Some(Percentage::from_ratio_f32(0.5)),
//!         volume: Percentage::from_ratio_f32(1.0),
//!         is_playing: true,
//!         is_shuffle: false,
//!         repeat_mode: RepeatMode::None,
//!     },
//! };
//! ```
//!
//! # Notes
//!
//! Device identifiers in the protocol can vary between platforms:
//! * iOS devices use uppercase UUIDs
//! * Android devices use lowercase UUIDs
//! * Some devices use other formats with prefixes
//!
//! For this reason, many IDs are handled as strings rather than parsed UUIDs.

use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fmt::{self, Write},
    io::Read,
    str::FromStr,
    time::Duration,
};

use base64::prelude::*;
use flate2::{
    Compression,
    read::{DeflateDecoder, DeflateEncoder},
};
use protobuf::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_repr::{Deserialize_repr, Serialize_repr};
use serde_with::{
    DeserializeFromStr, DisplayFromStr, DurationSeconds, NoneAsEmptyString, SerializeDisplay,
    formats::Flexible, json::JsonString, serde_as,
};
use uuid::Uuid;

use super::{channel::Ident, protos::queue};
use crate::{error::Error, protocol::Codec, track::TrackId};

/// A message's contents in the Deezer Connect protocol.
///
/// The `Contents` structure represents the complete data of a message, including:
/// * The message type identifier (`ident`)
/// * Routing headers (`headers`)
/// * The actual message payload (`body`)
///
/// # Wire Format
///
/// In the wire protocol, contents are serialized as JSON with this structure:
/// ```json
/// {
///     "APP": "REMOTECOMMAND",
///     "headers": {
///         "from": "device-uuid",
///         "destination": "target-uuid"
///     },
///     "body": {
///         "messageId": "msg-123",
///         "messageType": "playbackProgress",
///         "protocolVersion": "com.deezer.remote.command.proto1",
///         "payload": "base64-encoded-data",
///         "clock": {}
///     }
/// }
/// ```
///
/// The `body.payload` field can contain either:
/// * Base64-encoded JSON for most message types
/// * Base64-encoded, DEFLATE-compressed protocol buffers for queue data
///
/// # Examples
///
/// Creating playback progress contents:
/// ```rust
/// use std::time::Duration;
///
/// let contents = Contents {
///     ident: Ident::RemoteCommand,
///     headers: Headers {
///         from: DeviceId::default(),
///         destination: None,
///     },
///     body: Body::PlaybackProgress {
///         message_id: "msg123".to_string(),
///         track: QueueItem {
///             queue_id: "queue123".to_string(),
///             track_id: 12345.into(),
///             position: 0,
///         },
///         quality: AudioQuality::Standard,
///         duration: Duration::from_secs(180),
///         buffered: Duration::from_secs(10),
///         progress: Some(Percentage::from_ratio_f32(0.5)),
///         volume: Percentage::from_ratio_f32(1.0),
///         is_playing: true,
///         is_shuffle: false,
///         repeat_mode: RepeatMode::None,
///     },
/// };
/// ```
///
/// Creating device discovery contents:
/// ```rust
/// let contents = Contents {
///     ident: Ident::RemoteDiscover,
///     headers: Headers {
///         from: DeviceId::default(),
///         destination: None,
///     },
///     body: Body::DiscoveryRequest {
///         message_id: "msg456".to_string(),
///         from: DeviceId::default(),
///         discovery_session: "session789".to_string(),
///     },
/// };
/// ```
///
/// # Protocol Versions
///
/// Different message types use different protocol versions:
/// * `com.deezer.remote.command.proto1` - For playback control
/// * `com.deezer.remote.discovery.proto1` - For device discovery
/// * `com.deezer.remote.queue.proto1` - For queue management
///
/// # Display Format
///
/// When displayed using `Display`, contents show their message type with
/// fixed-width padding for consistent formatting:
/// ```text
/// PlaybackProgress
/// ConnectionOffer
/// DiscoveryRequest
/// ```
///
/// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Contents {
    /// The message type identifier.
    ///
    /// This field is serialized as "APP" in the wire format and determines
    /// how the message should be processed.
    #[serde(rename = "APP")]
    pub ident: Ident,

    /// Routing information for the message.
    ///
    /// Specifies the source device and optional destination device
    /// for the message.
    pub headers: Headers,

    /// The actual message payload.
    ///
    /// Contains the message-specific data, serialized according to the
    /// message type's requirements. The wire format embeds this as a JSON
    /// string, which is handled transparently by the serialization.
    #[serde_as(as = "JsonString")]
    pub body: Body,
}

/// Formats message contents for display, showing the message type with fixed-width padding.
///
/// This implementation provides a consistent display format for logging and debugging
/// purposes. The message type is left-aligned with 16 characters of padding.
///
/// # Examples
///
/// ```rust
/// let contents = Contents {
///     ident: Ident::RemoteCommand,
///     headers: Headers {
///         from: DeviceId::default(),
///         destination: None,
///     },
///     body: Body::PlaybackProgress { /* ... */ },
/// };
///
/// // Displays as: "PlaybackProgress "
/// println!("{}", contents);
/// ```
///
/// Different message types maintain consistent alignment:
/// ```text
/// PlaybackProgress
/// Ping
/// Stop
/// ```
///
/// # Notes
///
/// The current implementation has a known limitation where padding may not
/// be respected in all contexts. This is marked for future improvement.
impl fmt::Display for Contents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // FIXME: padding is not respected.
        write!(f, "{:<16}", self.body.message_type())
    }
}

/// Routing headers for Deezer Connect messages.
///
/// Headers contain the addressing information needed to route messages between
/// Deezer Connect devices. Each message must have a source device (`from`) and
/// may optionally specify a target device (`destination`).
///
/// # Device Identification
///
/// Devices are identified using various formats:
/// * Standard UUIDs with hyphens (e.g., "550e8400-e29b-41d4-a716-446655440000")
/// * UUIDs without hyphens
/// * Platform-specific formats (possibly including Android AAIDs or Apple IDFAs)
///
/// # Examples
///
/// Creating headers for broadcast messages:
/// ```rust
/// let headers = Headers {
///     from: DeviceId::default(),  // Creates a new random UUID
///     destination: None,          // No specific target
/// };
/// ```
///
/// Creating headers for targeted messages:
/// ```rust
/// use uuid::Uuid;
///
/// let headers = Headers {
///     from: DeviceId::Uuid(Uuid::new_v4()),
///     destination: Some(DeviceId::Other("target-device-123".to_string())),
/// };
/// ```
///
/// # Display Format
///
/// Headers are displayed in a human-readable format:
/// ```text
/// from 550e8400-e29b-41d4-a716-446655440000
/// from 550e8400-e29b-41d4-a716-446655440000 to target-device-123
/// ```
///
/// # Wire Format
///
/// In the protocol's JSON format:
/// ```json
/// {
///     "from": "550e8400-e29b-41d4-a716-446655440000",
///     "destination": "target-device-123"  // Optional
/// }
/// ```
///
/// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Headers {
    /// The source device identifier.
    ///
    /// This field is always present and identifies the device sending
    /// the message.
    pub from: DeviceId,

    /// The optional target device identifier.
    ///
    /// When `None`, the message is treated as a broadcast to all
    /// available devices.
    pub destination: Option<DeviceId>,
}

/// Formats the headers in a human-readable format.
///
/// The output format is:
/// * For broadcast: "from {device-id}"
/// * For targeted: "from {device-id} to {target-id}"
///
/// # Examples
///
/// ```rust
/// // Broadcast headers
/// let headers = Headers {
///     from: DeviceId::default(),
///     destination: None,
/// };
/// println!("{}", headers);  // "from 550e8400-e29b-41d4-a716-446655440000"
///
/// // Targeted headers
/// let headers = Headers {
///     from: DeviceId::default(),
///     destination: Some(DeviceId::Other("target-123".to_string())),
/// };
/// println!("{}", headers);  // "from 550e8400-e29b-41d4-a716-446655440000 to target-123"
/// ```
impl fmt::Display for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "from {}", self.from)?;

        if let Some(destination) = &self.destination {
            write!(f, " to {destination}")?;
        }

        Ok(())
    }
}

/// Device identifier in the Deezer Connect protocol.
///
/// A `DeviceId` can be either:
/// * A standard UUID (`Uuid` variant)
/// * A platform-specific identifier (`Other` variant)
///
/// # Platform Variations
///
/// Device identifiers vary by platform:
/// * iOS: Uppercase UUIDs with hyphens
/// * Android: Lowercase UUIDs without hyphens
/// * Web: UUIDs with hyphens
/// * Other platforms: Custom formats, possibly including:
///   * Android Advertising IDs (AAID)
///   * Apple IDFAs
///   * Custom prefixed identifiers
///
/// # Examples
///
/// Creating device IDs:
/// ```rust
/// use uuid::Uuid;
///
/// // Using UUID
/// let device = DeviceId::Uuid(Uuid::new_v4());
///
/// // Using platform-specific format
/// let device = DeviceId::Other("android-device-123".to_string());
/// ```
///
/// Parsing from strings:
/// ```rust
/// // Parse UUID format
/// let device: DeviceId = "550e8400-e29b-41d4-a716-446655440000".parse()?;
/// assert!(matches!(device, DeviceId::Uuid(_)));
///
/// // Parse other format
/// let device: DeviceId = "android-device-123".parse()?;
/// assert!(matches!(device, DeviceId::Other(_)));
/// ```
///
/// # Wire Format
///
/// Device IDs are serialized as simple strings:
/// ```text
/// "550e8400-e29b-41d4-a716-446655440000"  // UUID format
/// "android-device-123"                    // Other format
/// ```
#[derive(
    Clone, Debug, SerializeDisplay, DeserializeFromStr, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum DeviceId {
    /// A standard UUID identifier.
    ///
    /// This is the most common format, used by most platforms.
    Uuid(Uuid),

    /// A platform-specific identifier format.
    ///
    /// Used when the device ID doesn't conform to UUID format.
    Other(String),
}

/// Creates a new random UUID device identifier.
///
/// This is the recommended default for new devices joining
/// the protocol.
///
/// # Examples
///
/// ```rust
/// let device = DeviceId::default();
/// assert!(matches!(device, DeviceId::Uuid(_)));
/// ```
impl Default for DeviceId {
    fn default() -> Self {
        Self::Uuid(Uuid::new_v4())
    }
}

/// Creates a device ID from a UUID.
///
/// This provides convenient conversion from standard UUIDs.
///
/// # Examples
///
/// ```rust
/// use uuid::Uuid;
///
/// let uuid = Uuid::new_v4();
/// let device: DeviceId = uuid.into();
/// ```
impl From<Uuid> for DeviceId {
    fn from(uuid: Uuid) -> Self {
        Self::Uuid(uuid)
    }
}

/// Parses a device ID from its string representation.
///
/// The parser attempts to interpret the string as a UUID first.
/// If that fails, it treats it as a platform-specific format.
///
/// # Examples
///
/// ```rust
/// // Parse UUID format
/// let device: DeviceId = "550e8400-e29b-41d4-a716-446655440000".parse()?;
///
/// // Parse platform-specific format
/// let device: DeviceId = "android-device-123".parse()?;
///
/// // Both hyphenated and non-hyphenated UUIDs work
/// let device: DeviceId = "550e8400e29b41d4a716446655440000".parse()?;
/// ```
///
/// # Error Handling
///
/// This implementation never returns an error, as any string that isn't
/// a valid UUID is accepted as an `Other` variant.
impl FromStr for DeviceId {
    type Err = Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let device = match Uuid::try_parse(s) {
            Ok(uuid) => Self::from(uuid),
            Err(_) => Self::Other(s.to_owned()),
        };

        Ok(device)
    }
}

/// Formats a device ID for the wire protocol.
///
/// The output format depends on the variant:
/// * `Uuid`: Standard UUID string format
/// * `Other`: The platform-specific string as-is
///
/// # Examples
///
/// ```rust
/// use uuid::Uuid;
///
/// let uuid = Uuid::new_v4();
/// let device = DeviceId::Uuid(uuid);
/// assert_eq!(device.to_string(), uuid.to_string());
///
/// let device = DeviceId::Other("android-123".to_string());
/// assert_eq!(device.to_string(), "android-123");
/// ```
impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uuid(uuid) => write!(f, "{uuid}"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Message payload in the Deezer Connect protocol.
///
/// Represents the actual content of messages exchanged between devices. Each variant
/// corresponds to a specific message type and carries its relevant data.
///
/// # Wire Format Rules
///
/// Messages follow one of two formats depending on type:
///
/// JSON format (most messages):
/// ```json
/// {
///     "messageId": "msg-123",
///     "messageType": "playbackProgress",
///     "protocolVersion": "com.deezer.remote.command.proto1",
///     "payload": "base64-encoded-json",
///     "clock": {}
/// }
/// ```
///
/// Protocol Buffer format (queue messages):
/// ```json
/// {
///     "messageId": "msg-123",
///     "messageType": "publishQueue",
///     "protocolVersion": "com.deezer.remote.queue.proto1",
///     "payload": "base64-encoded-deflated-protobuf",
///     "clock": {}
/// }
/// ```
///
/// # Validation Rules
///
/// Common requirements for all messages:
/// * `messageId` must be non-empty
/// * `messageType` must match payload content
/// * `protocolVersion` must be supported for message type
///
/// Payload-specific rules:
/// * `PlaybackProgress`:
///   - Progress must be between 0.0 and 1.0
///   - Volume must be between 0.0 and 1.0
///   - Duration must be non-negative
///   - Queue ID must match track's queue
/// * `PublishQueue`:
///   - Queue must have valid ID
///   - Track positions must be sequential
///   - Track IDs must be valid
/// * `Skip`:
///   - Progress must be between 0.0 and 1.0 if present
///   - Volume must be between 0.0 and 1.0 if present
///   - Track must belong to specified queue if both present
///
/// # Examples
///
/// Valid messages:
/// ```rust
/// use std::time::Duration;
///
/// // Playback progress
/// let body = Body::PlaybackProgress {
///     message_id: "msg123".to_string(),
///     track: QueueItem {
///         queue_id: "queue123".to_string(),
///         track_id: 12345.into(),
///         position: 0,
///     },
///     quality: AudioQuality::Standard,
///     duration: Duration::from_secs(180),
///     buffered: Duration::from_secs(10),
///     progress: Some(Percentage::from_ratio_f32(0.5)),
///     volume: Percentage::from_ratio_f32(1.0),
///     is_playing: true,
///     is_shuffle: false,
///     repeat_mode: RepeatMode::None,
/// };
///
/// // Skip command
/// let body = Body::Skip {
///     message_id: "msg456".to_string(),
///     queue_id: Some("queue123".to_string()),
///     track: Some(queue_item),
///     progress: Some(Percentage::from_ratio_f32(0.0)),
///     should_play: Some(true),
///     set_repeat_mode: None,
///     set_shuffle: None,
///     set_volume: None,
/// };
/// ```
///
/// Error cases:
/// ```rust
/// use serde_json::json;
///
/// // Invalid progress value
/// let invalid = json!({
///     "messageId": "msg123",
///     "messageType": "playbackProgress",
///     "protocolVersion": "com.deezer.remote.command.proto1",
///     "payload": {
///         "progress": 1.5,  // Must be <= 1.0
///         // ... other fields ...
///     }
/// });
/// assert!(serde_json::from_value::<Body>(invalid).is_err());
///
/// // Mismatched message type
/// let invalid = json!({
///     "messageId": "msg123",
///     "messageType": "skip",
///     "protocolVersion": "com.deezer.remote.command.proto1",
///     "payload": {
///         // playbackProgress payload
///         "progress": 0.5,
///         // ... other fields ...
///     }
/// });
/// assert!(serde_json::from_value::<Body>(invalid).is_err());
///
/// // Invalid protocol version
/// let invalid = json!({
///     "messageId": "msg123",
///     "messageType": "playbackProgress",
///     "protocolVersion": "unknown",
///     "payload": {}
/// });
/// assert!(serde_json::from_value::<Body>(invalid).is_err());
///
/// // Queue with invalid track positions
/// let invalid = json!({
///     "messageId": "msg123",
///     "messageType": "publishQueue",
///     "protocolVersion": "com.deezer.remote.queue.proto1",
///     "payload": "base64-encoded-protobuf-with-gaps-in-positions"
/// });
/// assert!(serde_json::from_value::<Body>(invalid).is_err());
/// ```
///
/// # Protocol Versions
///
/// Different message types require specific protocol versions:
/// * `com.deezer.remote.command.proto1`:
///   - `PlaybackProgress`
///   - Skip
///   - Status
///   - etc.
/// * `com.deezer.remote.discovery.proto1`:
///   - Connect
///   - `ConnectionOffer`
///   - `DiscoveryRequest`
/// * `com.deezer.remote.queue.proto1`:
///   - `PublishQueue`
///   - `RefreshQueue`
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Acknowledges receipt of a message.
    Acknowledgement {
        /// Unique identifier for this message
        message_id: String,
        /// ID of the message being acknowledged
        acknowledgement_id: String,
    },

    /// Signals closure of a connection.
    Close {
        /// Unique identifier for this message
        message_id: String,
    },

    /// Initiates a connection to a device.
    Connect {
        /// Unique identifier for this message
        message_id: String,
        /// Device initiating the connection
        from: DeviceId,
        /// Optional connection offer ID to respond to
        offer_id: Option<String>,
    },

    /// Offers a connection to other devices.
    ConnectionOffer {
        /// Unique identifier for this message
        message_id: String,
        /// Device offering the connection
        from: DeviceId,
        /// Human-readable name of the device
        device_name: String,
        /// Type of device offering the connection
        device_type: DeviceType,
    },

    /// Requests device discovery.
    DiscoveryRequest {
        /// Unique identifier for this message
        message_id: String,
        /// Device initiating discovery
        from: DeviceId,
        /// Unique session identifier for this discovery
        discovery_session: String,
    },

    /// Reports playback status and progress.
    PlaybackProgress {
        /// Unique identifier for this message
        message_id: String,
        /// Currently playing track
        track: QueueItem,
        /// Audio quality of the currently playing track
        quality: AudioQuality,
        /// Total duration of the track
        duration: Option<Duration>,
        /// Amount of audio buffered from the start of the track
        buffered: Option<Duration>,
        /// Current playback position (0.0 to 1.0)
        progress: Option<Percentage>,
        /// Current volume level (0.0 to 1.0)
        volume: Percentage,
        /// Whether playback is active
        is_playing: bool,
        /// Whether shuffle mode is enabled
        is_shuffle: bool,
        /// Current repeat mode setting
        repeat_mode: RepeatMode,
    },

    /// Publishes a complete playback queue.
    PublishQueue {
        /// Unique identifier for this message
        message_id: String,
        /// The complete queue data
        queue: queue::List,
    },

    /// Network keep-alive message.
    Ping {
        /// Unique identifier for this message
        message_id: String,
    },

    /// Signals device readiness after completion of the connection handshake.
    Ready {
        /// Unique identifier for this message
        message_id: String,
    },

    /// Requests queue UI refresh.
    RefreshQueue {
        /// Unique identifier for this message
        message_id: String,
    },

    /// Controls playback state changes.
    Skip {
        /// Unique identifier for this message
        message_id: String,
        /// Target queue identifier
        queue_id: Option<String>,
        /// Track to skip to
        track: Option<QueueItem>,
        /// Position to seek to (0.0 to 1.0)
        progress: Option<Percentage>,
        /// Whether to start playing
        should_play: Option<bool>,
        /// New repeat mode setting
        set_repeat_mode: Option<RepeatMode>,
        /// New shuffle mode setting
        set_shuffle: Option<bool>,
        /// New volume level (0.0 to 1.0)
        set_volume: Option<Percentage>,
    },

    /// Reports command execution status.
    Status {
        /// Unique identifier for this message
        message_id: String,
        /// ID of the command being responded to
        command_id: String,
        /// Command execution result
        status: Status,
    },

    /// Requests playback stop.
    Stop {
        /// Unique identifier for this message
        message_id: String,
    },
}

impl Body {
    /// Returns the message type of this body.
    ///
    /// This method provides the type identifier used in the wire protocol
    /// to determine how the message should be processed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let body = Body::Ping {
    ///     message_id: "msg123".to_string()
    /// };
    /// assert_eq!(body.message_type(), MessageType::Ping);
    ///
    /// let body = Body::Stop {
    ///     message_id: "msg456".to_string()
    /// };
    /// assert_eq!(body.message_type(), MessageType::Stop);
    /// ```
    #[must_use]
    pub fn message_type(&self) -> MessageType {
        match self {
            Self::Acknowledgement { .. } => MessageType::Acknowledgement,
            Self::Close { .. } => MessageType::Close,
            Self::Connect { .. } => MessageType::Connect,
            Self::ConnectionOffer { .. } => MessageType::ConnectionOffer,
            Self::DiscoveryRequest { .. } => MessageType::DiscoveryRequest,
            Self::PlaybackProgress { .. } => MessageType::PlaybackProgress,
            Self::PublishQueue { .. } => MessageType::PublishQueue,
            Self::Ping { .. } => MessageType::Ping,
            Self::Ready { .. } => MessageType::Ready,
            Self::RefreshQueue { .. } => MessageType::RefreshQueue,
            Self::Skip { .. } => MessageType::Skip,
            Self::Status { .. } => MessageType::Status,
            Self::Stop { .. } => MessageType::Stop,
        }
    }

    /// Returns the unique message identifier for this body.
    ///
    /// Every message in the Deezer Connect protocol has a unique identifier
    /// that can be used for acknowledgments and correlation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let body = Body::Ping {
    ///     message_id: "msg123".to_string()
    /// };
    /// assert_eq!(body.message_id(), "msg123");
    ///
    /// let body = Body::PlaybackProgress {
    ///     message_id: "msg456".to_string(),
    ///     // ... other fields ...
    /// };
    /// assert_eq!(body.message_id(), "msg456");
    /// ```
    #[must_use]
    pub fn message_id(&self) -> &str {
        match self {
            Self::Acknowledgement { message_id, .. }
            | Self::Close { message_id, .. }
            | Self::Connect { message_id, .. }
            | Self::ConnectionOffer { message_id, .. }
            | Self::DiscoveryRequest { message_id, .. }
            | Self::PlaybackProgress { message_id, .. }
            | Self::PublishQueue { message_id, .. }
            | Self::Ping { message_id, .. }
            | Self::Ready { message_id, .. }
            | Self::RefreshQueue { message_id, .. }
            | Self::Skip { message_id, .. }
            | Self::Status { message_id, .. }
            | Self::Stop { message_id, .. } => message_id,
        }
    }
}

/// Command execution status in the Deezer Connect protocol.
///
/// Represents the result of command execution, with a default assumption
/// of failure unless explicitly marked as successful.
///
/// # Protocol Representation
///
/// In the wire format, statuses are represented as integers:
/// * `0` - Success (`OK`)
/// * `1` - Failure (`Error`)
///
/// # Default Behavior
///
/// Following the protocol's defensive approach, the default status is `Error`.
/// This ensures that success must be explicitly indicated rather than assumed.
///
/// # Examples
///
/// ```rust
/// // Creating status values
/// let success = Status::OK;
/// let failure = Status::Error;
///
/// // Default is Error
/// let default_status = Status::default();
/// assert_eq!(default_status, Status::Error);
///
/// // Serialization to integer values
/// use serde_json;
/// assert_eq!(serde_json::to_string(&Status::OK)?, "0");
/// assert_eq!(serde_json::to_string(&Status::Error)?, "1");
/// ```
///
/// Using in command responses:
/// ```rust
/// let body = Body::Status {
///     message_id: "msg123".to_string(),
///     command_id: "cmd456".to_string(),
///     status: Status::OK,
/// };
/// ```
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Hash,
    Serialize_repr,
    Deserialize_repr,
    PartialOrd,
    Ord,
    PartialEq,
    Eq,
)]
#[repr(u64)]
pub enum Status {
    /// Command executed successfully.
    OK = 0,

    /// Command execution failed.
    ///
    /// This is the default status, following the protocol's
    /// "fail-by-default" approach.
    #[default]
    Error = 1,
}

/// Formats the status for human-readable output.
///
/// # Examples
///
/// ```rust
/// assert_eq!(Status::OK.to_string(), "Ok");
/// assert_eq!(Status::Error.to_string(), "Err");
///
/// // Useful for logging
/// println!("Command completed with status: {}", Status::OK);
/// ```
impl fmt::Display for Status {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::OK => write!(f, "Ok"),
            Status::Error => write!(f, "Err"),
        }
    }
}

/// Playback repeat mode in the Deezer Connect protocol.
///
/// Controls how playback continues after reaching the end of the current track
/// or queue. The protocol represents these modes as integers in the wire format.
///
/// # Wire Format Values
///
/// * `0` - No repeat (`None`)
/// * `1` - Repeat entire queue (`All`)
/// * `2` - Repeat current track (`One`)
/// * `-1` - Unrecognized mode
///
/// # Examples
///
/// Basic usage:
/// ```rust
/// // Default is no repeat
/// let mode = RepeatMode::default();
/// assert_eq!(mode, RepeatMode::None);
///
/// // Setting different modes
/// let repeat_all = RepeatMode::All;
/// let repeat_one = RepeatMode::One;
/// ```
///
/// In playback progress messages:
/// ```rust
/// let body = Body::PlaybackProgress {
///     message_id: "msg123".to_string(),
///     // ... other fields ...
///     repeat_mode: RepeatMode::All,
///     // ... other fields ...
/// };
/// ```
///
/// Wire format serialization:
/// ```rust
/// use serde_json;
///
/// assert_eq!(serde_json::to_string(&RepeatMode::None)?, "0");
/// assert_eq!(serde_json::to_string(&RepeatMode::All)?, "1");
/// assert_eq!(serde_json::to_string(&RepeatMode::One)?, "2");
/// assert_eq!(serde_json::to_string(&RepeatMode::Unrecognized)?, "-1");
/// ```
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Hash,
    Serialize_repr,
    Deserialize_repr,
    PartialOrd,
    Ord,
    PartialEq,
    Eq,
)]
// `i64` because this is serialized into and deserialized from JSON.
#[repr(i64)]
pub enum RepeatMode {
    /// No repeat - play through queue once.
    ///
    /// This is the default mode.
    #[default]
    None = 0,

    /// Repeat entire queue - restart from beginning after last track.
    All = 1,

    /// Repeat current track - play same track repeatedly.
    One = 2,

    /// Unknown or unsupported repeat mode.
    Unrecognized = -1,
}

/// Formats the repeat mode for human-readable output.
///
/// # Examples
///
/// ```rust
/// assert_eq!(RepeatMode::None.to_string(), "None");
/// assert_eq!(RepeatMode::All.to_string(), "All");
/// assert_eq!(RepeatMode::One.to_string(), "One");
/// assert_eq!(RepeatMode::Unrecognized.to_string(), "Unrecognized");
impl fmt::Display for RepeatMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepeatMode::None => write!(f, "None"),
            RepeatMode::All => write!(f, "All"),
            RepeatMode::One => write!(f, "One"),
            RepeatMode::Unrecognized => write!(f, "Unknown"),
        }
    }
}

#[expect(clippy::doc_markdown)]
/// # Quality Levels
///
/// * `Basic` - 64 kbps constant bitrate MP3
/// * `Standard` - 128 kbps constant bitrate MP3 (default)
/// * `High` - 320 kbps constant bitrate MP3 (Premium subscription)
/// * `Lossless` - FLAC lossless compression (HiFi subscription)
/// * `Unknown` - Unrecognized quality level
///
/// # Bitrates
///
/// Use the [`bitrate`](Self::bitrate) method to get the nominal bitrate
/// for each quality level in kbps. MP3 formats use constant bitrates,
/// while FLAC uses variable bitrate compression - its actual bitrate
/// varies depending on the audio content's complexity, typically much
/// lower than the theoretical maximum of 1411 kbps for 16-bit/44.1kHz
/// stereo audio.
///
/// # Subscription Requirements
///
/// Different quality levels require specific subscription tiers:
/// * Free users: Basic and Standard
/// * Premium: Up to High Quality
/// * HiFi: All quality levels including Lossless
///
/// # Wire Format
///
/// Quality levels are represented as integers in the protocol:
/// * `0` - Basic
/// * `1` - Standard (default)
/// * `2` - High
/// * `3` - Lossless
/// * `-1` - Unknown
///
/// # Examples
///
/// Basic usage:
/// ```rust
/// // Default is Standard quality
/// let quality = AudioQuality::default();
/// assert_eq!(quality, AudioQuality::Standard);
///
/// // Different quality levels
/// let basic = AudioQuality::Basic;
/// let hifi = AudioQuality::Lossless;
/// ```
///
/// In playback progress messages:
/// ```rust
/// let body = Body::PlaybackProgress {
///     message_id: "msg123".to_string(),
///     // ... other fields ...
///     quality: AudioQuality::High,
///     // ... other fields ...
/// };
/// ```
///
/// String parsing:
/// ```rust
/// assert_eq!("low".parse::<AudioQuality>()?, AudioQuality::Basic);
/// assert_eq!("standard".parse::<AudioQuality>()?, AudioQuality::Standard);
/// assert_eq!("high".parse::<AudioQuality>()?, AudioQuality::High);
/// assert_eq!("lossless".parse::<AudioQuality>()?, AudioQuality::Lossless);
/// assert_eq!("unknown".parse::<AudioQuality>()?, AudioQuality::Unknown);
/// ```
///
/// Wire format serialization:
/// ```rust
/// use serde_json;
///
/// assert_eq!(serde_json::to_string(&AudioQuality::Basic)?, "0");
/// assert_eq!(serde_json::to_string(&AudioQuality::Standard)?, "1");
/// assert_eq!(serde_json::to_string(&AudioQuality::High)?, "2");
/// assert_eq!(serde_json::to_string(&AudioQuality::Lossless)?, "3");
/// assert_eq!(serde_json::to_string(&AudioQuality::Unknown)?, "-1");
/// ```
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Hash,
    Serialize_repr,
    Deserialize_repr,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
// `i64` because this is serialized into and deserialized from JSON.
#[repr(i64)]
pub enum AudioQuality {
    /// 64 kbps MP3 quality.
    ///
    /// Available to all users.
    Basic = 0,

    /// 128 kbps MP3 quality.
    ///
    /// This is the default quality level.
    #[default]
    Standard = 1,

    /// 320 kbps MP3 quality.
    ///
    /// Requires Premium subscription.
    High = 2,

    #[expect(clippy::doc_markdown)]
    /// 1411 kbps FLAC quality.
    ///
    /// Requires HiFi subscription.
    Lossless = 3,

    /// Unknown or unrecognized quality level.
    Unknown = -1,
}

impl AudioQuality {
    #[expect(clippy::doc_markdown)]
    /// Audio quality levels in the Deezer Connect protocol.
    ///
    /// Represents the different audio quality tiers available in Deezer,
    /// corresponding to different codecs and bitrates. Note that the remote
    /// device cannot control the audio quality - it can only report it.
    ///
    /// # Quality Levels
    ///
    /// * `Basic` - MP3 at 64 kbps constant bitrate
    /// * `Standard` - MP3 at 128 kbps constant bitrate (default)
    /// * `High` - MP3 at 320 kbps constant bitrate (Premium subscription)
    /// * `Lossless` - FLAC variable bitrate compression (HiFi subscription)
    /// * `Unknown` - Unrecognized quality level
    ///
    /// # Format Information
    ///
    /// Use the following methods to get format details:
    /// * [`codec`](Self::codec) - Get the audio codec (MP3 or FLAC)
    /// * [`bitrate`](Self::bitrate) - Get the bitrate in kbps
    ///
    /// Note that while MP3 formats use constant bitrates, FLAC uses variable
    /// bitrate compression - its actual bitrate varies with audio content
    /// complexity, typically much lower than the maximum of 1411 kbps for
    /// 16-bit/44.1kHz stereo audio.
    #[must_use]
    pub fn bitrate(&self) -> Option<usize> {
        let bitrate = match self {
            AudioQuality::Unknown => return None,
            AudioQuality::Basic => 64,
            AudioQuality::Standard => 128,
            AudioQuality::High => 320,
            AudioQuality::Lossless => 1411,
        };

        Some(bitrate)
    }

    /// Returns the audio codec name for this quality level.
    ///
    /// # Returns
    ///
    /// * `Some("MP3")` - For Basic, Standard, and High quality (constant bitrate)
    /// * `Some("FLAC")` - For Lossless quality (variable bitrate)
    /// * `None` - For Unknown quality
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(AudioQuality::Basic.codec(), Some("MP3"));
    /// assert_eq!(AudioQuality::Standard.codec(), Some("MP3"));
    /// assert_eq!(AudioQuality::High.codec(), Some("MP3"));
    /// assert_eq!(AudioQuality::Lossless.codec(), Some("FLAC"));
    /// assert_eq!(AudioQuality::Unknown.codec(), None);
    /// ```
    #[must_use]
    pub fn codec(&self) -> Option<Codec> {
        let codec = match self {
            AudioQuality::Unknown => return None,
            AudioQuality::Lossless => Codec::FLAC,
            _ => Codec::MP3,
        };

        Some(codec)
    }
}

/// Formats the audio quality for human-readable output.
///
/// # Examples
///
/// ```rust
/// assert_eq!(AudioQuality::Basic.to_string(), "Basic");
/// assert_eq!(AudioQuality::Standard.to_string(), "Standard");
/// assert_eq!(AudioQuality::High.to_string(), "High Quality");
/// assert_eq!(AudioQuality::Lossless.to_string(), "High Fidelity");
/// assert_eq!(AudioQuality::Unknown.to_string(), "Unknown");
/// ```
impl fmt::Display for AudioQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioQuality::Basic => write!(f, "Basic"),
            AudioQuality::Standard => write!(f, "Standard"),
            AudioQuality::High => write!(f, "High Quality"),
            AudioQuality::Lossless => write!(f, "High Fidelity"),
            AudioQuality::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Parses a string into an audio quality level.
///
/// Accepts common quality level names and aliases.
///
/// # Examples
///
/// ```rust
/// assert_eq!("low".parse()?, AudioQuality::Basic);
/// assert_eq!("standard".parse()?, AudioQuality::Standard);
/// assert_eq!("high".parse()?, AudioQuality::High);
/// assert_eq!("lossless".parse()?, AudioQuality::Lossless);
///
/// // Unknown values parse to Unknown
/// assert_eq!("invalid".parse()?, AudioQuality::Unknown);
/// ```
impl FromStr for AudioQuality {
    type Err = Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let variant = match s {
            "low" => AudioQuality::Basic,
            "standard" => AudioQuality::Standard,
            "high" => AudioQuality::High,
            "lossless" => AudioQuality::Lossless,
            _ => AudioQuality::Unknown,
        };

        Ok(variant)
    }
}

/// Represents a ratio or percentage value in the Deezer Connect protocol.
///
/// This type stores values as ratios (0.0 to 1.0) but can display them as
/// percentages (0% to 100%). Used for various measurements including:
/// * Playback progress
/// * Volume levels
/// * Buffer status
///
/// # Internal Representation
///
/// Values are stored internally as `f32` ratios between 0.0 and 1.0, but the
/// type provides methods to work with both ratio and percentage formats.
///
/// # Constants
///
/// Provides common values as const:
/// ```rust
/// const ZERO: Percentage = Percentage::ZERO;         // 0%
/// const MAX: Percentage = Percentage::ONE_HUNDRED;   // 100%
/// ```
///
/// # Examples
///
/// Creating percentages:
/// ```rust
/// // Using const constructors
/// const HALF: Percentage = Percentage::from_ratio(0.5);
/// const QUARTER: Percentage = Percentage::from_percent(25.0);
///
/// // Using runtime constructors
/// let progress = Percentage::from_ratio(0.75);
/// let volume = Percentage::from_percent(80.0);
/// ```
///
/// Using in messages:
/// ```rust
/// let body = Body::PlaybackProgress {
///     // ...
///     progress: Some(Percentage::from_ratio(0.25)), // 25% through track
///     volume: Percentage::from_ratio(0.8),          // 80% volume
///     // ...
/// };
/// ```
///
/// Display formatting:
/// ```rust
/// let progress = Percentage::from_ratio(0.753);
/// assert_eq!(progress.to_string(), "75.3%");
/// ```
///
/// # Serialization
///
/// When serialized, the value is represented as its ratio (0.0 to 1.0):
/// ```rust
/// use serde_json;
///
/// let half = Percentage::from_ratio(0.5);
/// assert_eq!(serde_json::to_string(&half)?, "0.5");
/// ```
///
/// /// # Floating-Point Comparisons
///
/// The `PartialEq` implementation uses a relative epsilon comparison method for
/// floating-point values to handle numerical imprecision.
///
/// This ensures that values that are mathematically equal but may have slight
/// floating-point representation differences (like 0.1 + 0.2 and 0.3) are
/// considered equal. This is particularly important when comparing volumes and
/// progress values that may have undergone multiple calculations.
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize, PartialOrd)]
pub struct Percentage(f32);

impl Percentage {
    /// Represents 0% (0.0)
    ///
    /// # Examples
    ///
    /// ```rust
    /// const MUTED: Percentage = Percentage::ZERO;
    /// assert_eq!(MUTED.as_percent(), 0.0);
    /// ```
    pub const ZERO: Self = Self(0.0);

    /// Represents 100% (1.0)
    ///
    /// # Examples
    ///
    /// ```rust
    /// const MAX_VOLUME: Percentage = Percentage::ONE_HUNDRED;
    /// assert_eq!(MAX_VOLUME.as_percent(), 100.0);
    /// ```
    pub const ONE_HUNDRED: Self = Self(1.0);

    /// Creates a new percentage from a 32-bit floating point ratio.
    ///
    /// Can be used in const contexts.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Const context
    /// const HALF: Percentage = Percentage::from_ratio(0.5);
    /// assert_eq!(HALF.as_percent(), 50.0);
    ///
    /// // Runtime context
    /// let p = Percentage::from_ratio(0.75);
    /// assert_eq!(p.as_percent(), 75.0);
    /// ```
    #[must_use]
    #[inline]
    pub const fn from_ratio(ratio: f32) -> Self {
        Self(ratio)
    }

    /// Creates a new percentage from a 32-bit floating point percentage value.
    ///
    /// Can be used in const contexts.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Const context
    /// const HALF: Percentage = Percentage::from_percent(50.0);
    /// assert_eq!(HALF.as_ratio(), 0.5);
    ///
    /// // Runtime context
    /// let p = Percentage::from_percent(75.0);
    /// assert_eq!(p.as_ratio(), 0.75);
    /// ```
    #[must_use]
    #[inline]
    pub const fn from_percent(percent: f32) -> Self {
        Self(percent / 100.0)
    }

    /// Returns the value as a 32-bit floating point ratio (0.0 to 1.0).
    ///
    /// # Examples
    ///
    /// ```rust
    /// let p = Percentage::from_ratio(0.75);
    /// assert_eq!(p.as_ratio(), 0.75);
    /// ```
    #[must_use]
    pub fn as_ratio(&self) -> f32 {
        self.0
    }

    /// Returns the value as a 32-bit floating point percentage (0.0 to 100.0).
    ///
    /// # Examples
    ///
    /// ```rust
    /// let p = Percentage::from_ratio(0.75);
    /// assert_eq!(p.as_percent(), 75.0);
    /// ```
    #[must_use]
    pub fn as_percent(&self) -> f32 {
        self.0 * 100.0
    }
}

/// Compares two percentages using a relative epsilon comparison method.
///
/// This implementation uses a relative epsilon comparison for floating-point values
/// to handle numerical imprecision. The formula:
/// `2.0 * |a - b| <= ε * (|a| + |b|)`
/// accounts for the magnitude of the values being compared.
///
/// # Examples
/// ```rust
/// let p1 = Percentage::from_ratio(0.1 + 0.2);
/// let p2 = Percentage::from_ratio(0.3);
/// assert_eq!(p1, p2);  // Equal despite floating-point arithmetic
///
/// let p3 = Percentage::from_ratio(0.5);
/// let p4 = Percentage::from_ratio(0.5000001);
/// assert_eq!(p3, p4);  // Equal within epsilon
/// ```
impl PartialEq for Percentage {
    fn eq(&self, other: &Self) -> bool {
        2.0 * (self.0 - other.0).abs() <= f32::EPSILON * (self.0.abs() + other.0.abs())
    }
}

/// Formats the value as a percentage with one decimal place.
///
/// The output format is "XX.X%" (e.g., "75.3%").
///
/// # Examples
///
/// ```rust
/// let p = Percentage::from_ratio(0.753);
/// assert_eq!(p.to_string(), "75.3%");
///
/// let p = Percentage::from_ratio(1.0);
/// assert_eq!(p.to_string(), "100.0%");
/// ```
impl fmt::Display for Percentage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}%", self.as_percent())
    }
}

/// Represents an item in a Deezer Connect playback queue.
///
/// A queue item combines:
/// * The queue's unique identifier
/// * The track's Deezer ID
/// * The track's position in the queue
///
/// # Wire Format
///
/// Queue items are serialized as hyphen-separated strings:
/// ```text
/// <queue-uuid>-<track-id>-<position>
/// ```
///
/// Special cases:
/// * User-uploaded tracks have negative track IDs
/// * Queue UUIDs must be valid UUID strings
/// * Position is zero-based
///
/// # Examples
///
/// Creating a queue item:
/// ```rust
/// let item = QueueItem {
///     queue_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
///     track_id: 12345.into(),
///     position: 0,
/// };
/// ```
///
/// Serialization:
/// ```rust
/// // Normal track
/// let item = QueueItem {
///     queue_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
///     track_id: 12345.into(),
///     position: 0,
/// };
/// assert_eq!(item.to_string(), "550e8400-e29b-41d4-a716-446655440000-12345-0");
///
/// // User-uploaded track
/// let item = QueueItem {
///     queue_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
///     track_id: (-12345).into(),
///     position: 1,
/// };
/// assert_eq!(item.to_string(), "550e8400-e29b-41d4-a716-446655440000--12345-1");
/// ```
///
/// Parsing from string:
/// ```rust
/// let s = "550e8400-e29b-41d4-a716-446655440000-12345-0";
/// let item: QueueItem = s.parse()?;
///
/// assert_eq!(item.queue_id, "550e8400-e29b-41d4-a716-446655440000");
/// assert_eq!(item.track_id, 12345.into());
/// assert_eq!(item.position, 0);
/// ```
#[derive(
    Clone, Debug, SerializeDisplay, DeserializeFromStr, PartialOrd, Ord, PartialEq, Eq, Hash,
)]
pub struct QueueItem {
    /// The unique identifier of the queue.
    ///
    /// Must be a valid UUID string.
    pub queue_id: String,

    /// The Deezer track identifier.
    ///
    /// Can be either:
    /// * Positive - Normal Deezer tracks
    /// * Negative - User-uploaded tracks
    pub track_id: TrackId,

    /// Zero-based position in the queue.
    ///
    /// This value is used to index into the queue array and must
    /// be less than the queue length.
    // `usize` because this will index into an array. Also from the protobuf it
    // is known that this really an `u32`.
    pub position: usize,
}

impl QueueItem {
    /// Separator character used in the wire format.
    ///
    /// Used to split queue items into their components when parsing
    /// and to join components when serializing.
    const SEPARATOR: char = '-';
}

/// Formats a queue item for wire protocol transmission.
///
/// The output format is: `<queue-uuid>-<track-id>-<position>`
///
/// # Examples
///
/// ```rust
/// let item = QueueItem {
///     queue_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
///     track_id: 12345.into(),
///     position: 0,
/// };
///
/// assert_eq!(item.to_string(), "550e8400-e29b-41d4-a716-446655440000-12345-0");
/// ```
impl fmt::Display for QueueItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}{}{}",
            self.queue_id,
            Self::SEPARATOR,
            self.track_id,
            Self::SEPARATOR,
            self.position
        )
    }
}

/// Parses a queue item from its wire format string.
///
/// # Format
///
/// Expects a string in the format: `<queue-uuid>-<track-id>-<position>`
///
/// # Examples
///
/// ```rust
/// // Normal track
/// let s = "550e8400-e29b-41d4-a716-446655440000-12345-0";
/// let item: QueueItem = s.parse()?;
///
/// // User-uploaded track
/// let s = "550e8400-e29b-41d4-a716-446655440000--12345-1";
/// let item: QueueItem = s.parse()?;
/// ```
///
/// # Errors
///
/// Returns an error if:
/// * The queue ID is not a valid UUID
/// * The track ID is not a valid integer
/// * The position is not a valid integer
/// * The string format is invalid
impl FromStr for QueueItem {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut parts = s.split(Self::SEPARATOR);

        // Queue ID must be reconstructed from first 5 parts (UUID format)
        let mut queue_id = String::new();
        for i in 0..5 {
            match parts.next() {
                Some(part) => {
                    write!(queue_id, "{part}")?;
                    if i < 4 {
                        write!(queue_id, "{}", Self::SEPARATOR)?;
                    }
                }
                None => {
                    return Err(Self::Err::invalid_argument(format!(
                        "list element string slice should hold five `queue_id` parts, found {i}"
                    )));
                }
            }
        }

        // Validate the queue ID is a proper UUID
        if let Err(e) = Uuid::try_parse(&queue_id) {
            return Err(Self::Err::invalid_argument(format!("queue id: {e}")));
        }

        // Parse track ID, handling user-uploaded tracks (negative IDs)
        let track_id = parts.next().ok_or_else(|| {
            Self::Err::invalid_argument(
                "list element string slice should hold `track_id` part".to_string(),
            )
        })?;

        let track_id = if track_id.is_empty() {
            if let Some(user_uploaded_id) = parts.next() {
                let negative_track_id = format!("-{user_uploaded_id}");
                negative_track_id.parse::<TrackId>()?
            } else {
                return Err(Self::Err::invalid_argument(
                    "user-uploaded track id should not be empty".to_string(),
                ));
            }
        } else {
            track_id.parse::<TrackId>()?
        };

        // Parse position
        let position = parts.next().ok_or_else(|| {
            Self::Err::invalid_argument(
                "list element string slice should hold `position` part".to_string(),
            )
        })?;
        let position = position.parse::<usize>()?;

        Ok(Self {
            queue_id,
            track_id,
            position,
        })
    }
}

/// Serializes a message body for wire transmission.
///
/// This implementation converts the `Body` into a [`WireBody`] before
/// serialization to ensure proper wire format encoding. The process handles:
/// * JSON encoding for most messages
/// * Protocol Buffer encoding for queue messages
/// * Base64 encoding of payloads
/// * DEFLATE compression when required
///
/// # Examples
///
/// ```rust
/// let body = Body::Ping {
///     message_id: "msg123".to_string()
/// };
///
/// let json = serde_json::to_string(&body)?;
/// // Results in a wire format message with Base64-encoded payload
/// ```
///
/// [JSON]: https://www.json.org/
/// [`WireBody`]: struct.WireBody.html
// For syntactic sugar this could be changed into `serde_with::SerializeAs` but
// this now follows the same idiom as serializing a `Message`.
impl Serialize for Body {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let wire_body = WireBody::from(self.clone());
        wire_body.serialize(serializer)
    }
}

/// Deserializes a message body from wire format.
///
/// This implementation first deserializes into a [`WireBody`], then
/// converts it into the appropriate `Body` variant. The process handles:
/// * Base64 decoding of payloads
/// * JSON parsing for most messages
/// * Protocol Buffer parsing for queue messages
/// * DEFLATE decompression when required
///
/// # Examples
///
/// ```rust
/// // Wire format JSON with Base64-encoded payload
/// let json = r#"{
///     "messageId": "msg123",
///     "messageType": "ping",
///     "protocolVersion": "com.deezer.remote.command.proto1",
///     "payload": "base64data",
///     "clock": {}
/// }"#;
///
/// let body: Body = serde_json::from_str(json)?;
/// assert!(matches!(body, Body::Ping { .. }));
/// ```
///
/// # Errors
///
/// Returns an error if:
/// * The wire format is invalid
/// * Base64 decoding fails
/// * JSON parsing fails
/// * Protocol Buffer parsing fails
/// * Message type is unknown
///
/// [JSON]: https://www.json.org/
/// [`WireBody`]: struct.WireBody.html
// For syntactic sugar this could be changed into `serde_with::DeserializeAs` but
// this now follows the same idiom as deserializing a `Message`.
impl<'de> Deserialize<'de> for Body {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let wire_body = WireBody::deserialize(deserializer)?;
        Self::try_from(wire_body).map_err(serde::de::Error::custom)
    }
}

/// Internal wire format representation of message bodies in Deezer Connect.
///
/// This type handles the low-level protocol format, acting as an intermediate
/// representation between the high-level [`Body`] type and the wire protocol.
/// It manages protocol versioning, payload encoding, and message type routing.
///
/// # Wire Format
///
/// Messages are serialized as JSON with this structure:
/// ```json
/// {
///     "messageId": "msg-123",
///     "messageType": "playbackProgress",
///     "protocolVersion": "com.deezer.remote.command.proto1",
///     "payload": "base64-encoded-data",
///     "clock": {}
/// }
/// ```
///
/// # Protocol Versions
///
/// The protocol supports three distinct versions:
/// * `com.deezer.remote.command.proto1` - Playback control messages
/// * `com.deezer.remote.discovery.proto1` - Device discovery messages
/// * `com.deezer.remote.queue.proto1` - Queue management messages
///
/// # Payload Encoding
///
/// Payloads can be encoded in two ways:
/// * Base64-encoded JSON for most messages
/// * Base64-encoded, DEFLATE-compressed Protocol Buffers for queue data
///
/// # Examples
///
/// ```rust
/// let wire_body = WireBody {
///     message_id: "msg123".to_string(),
///     message_type: MessageType::PlaybackProgress,
///     protocol_version: WireBody::COMMAND_VERSION.to_string(),
///     payload: Payload::PlaybackProgress {
///         queue_id: "queue123".to_string(),
///         element_id: queue_item,
///         duration: Duration::from_secs(180),
///         buffered: Duration::from_secs(10),
///         progress: Some(Percentage::from_ratio_f32(0.5)),
///         volume: Percentage::from_ratio_f32(1.0),
///         quality: AudioQuality::Standard,
///         is_playing: true,
///         is_shuffle: false,
///         repeat_mode: RepeatMode::None,
///     },
///     clock: HashMap::new(),
/// };
/// ```
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WireBody {
    /// Unique message identifier.
    ///
    /// Used for message correlation and acknowledgments.
    message_id: String,

    /// Type of message being transmitted.
    ///
    /// Determines how the payload should be interpreted.
    message_type: MessageType,

    /// Protocol version for this message.
    ///
    /// Must match one of the known protocol versions.
    protocol_version: String,

    /// Message-specific data.
    ///
    /// Encoded according to message type requirements.
    #[serde_as(as = "DisplayFromStr")]
    payload: Payload,

    /// Reserved field for future use.
    ///
    /// Currently always empty. Maintained for protocol compatibility.
    clock: HashMap<String, serde_json::Value>,
}

/// Message type identifiers in the Deezer Connect protocol.
///
/// Each variant represents a distinct type of message that can be exchanged
/// between Deezer Connect devices. The type determines:
/// * Protocol version to use
/// * Expected payload format
/// * Message handling rules
///
/// # Wire Format
///
/// Message types are serialized as camelCase strings in JSON, with one
/// special case:
/// * `"ack"` for Acknowledgement
/// * All others are direct camelCase versions of their variant names
///
/// # Protocol Versions
///
/// Message types are grouped by protocol version:
///
/// Command Protocol (`com.deezer.remote.command.proto1`):
/// * `Acknowledgement`
/// * `Close`
/// * `PlaybackProgress`
/// * `Ping`
/// * `Ready`
/// * `Skip`
/// * `Status`
/// * `Stop`
///
/// Discovery Protocol (`com.deezer.remote.discovery.proto1`):
/// * `Connect`
/// * `ConnectionOffer`
/// * `DiscoveryRequest`
///
/// Queue Protocol (`com.deezer.remote.queue.proto1`):
/// * `PublishQueue`
/// * `RefreshQueue`
///
/// # Examples
///
/// ```rust
/// // Wire format serialization
/// use serde_json;
///
/// assert_eq!(
///     serde_json::to_string(&MessageType::Acknowledgement)?,
///     r#""ack""#
/// );
/// assert_eq!(
///     serde_json::to_string(&MessageType::PlaybackProgress)?,
///     r#""playbackProgress""#
/// );
///
/// // Protocol usage
/// let wire_body = WireBody {
///     message_type: MessageType::PlaybackProgress,
///     protocol_version: WireBody::COMMAND_VERSION.to_string(),
///     // ... other fields ...
/// };
/// ```
///
/// # Display Format
///
/// When displayed, message types use their variant names:
/// ```rust
/// assert_eq!(MessageType::Ping.to_string(), "Ping");
/// assert_eq!(MessageType::PlaybackProgress.to_string(), "PlaybackProgress");
/// ```
#[derive(Copy, Clone, Debug, Hash, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum MessageType {
    /// Confirms message receipt.
    ///
    /// Wire format: `"ack"`
    #[serde(rename = "ack")]
    Acknowledgement,

    /// Signals connection termination.
    ///
    /// Wire format: `"close"`
    Close,

    /// Initiates device connection.
    ///
    /// Wire format: `"connect"`
    Connect,

    /// Offers connection to other devices.
    ///
    /// Wire format: `"connectionOffer"`
    ConnectionOffer,

    /// Requests device discovery.
    ///
    /// Wire format: `"discoveryRequest"`
    DiscoveryRequest,

    /// Reports playback state and progress.
    ///
    /// Wire format: `"playbackProgress"`
    PlaybackProgress,

    /// Publishes complete queue contents.
    ///
    /// Wire format: `"publishQueue"`
    PublishQueue,

    /// Network keep-alive message.
    ///
    /// Wire format: `"ping"`
    Ping,

    /// Signals device readiness.
    ///
    /// Wire format: `"ready"`
    Ready,

    /// Requests queue UI refresh.
    ///
    /// Wire format: `"refreshQueue"`
    RefreshQueue,

    /// Controls playback state changes.
    ///
    /// Wire format: `"skip"`
    Skip,

    /// Reports command execution status.
    ///
    /// Wire format: `"status"`
    Status,

    /// Requests playback stop.
    ///
    /// Wire format: `"stop"`
    Stop,
}

/// Message payload data in the Deezer Connect protocol.
///
/// Represents the various types of data that can be carried in messages.
/// Payloads are encoded differently depending on their type:
/// * JSON-based payloads are Base64-encoded
/// * Protocol Buffer payloads are DEFLATE-compressed and Base64-encoded
/// * Empty payloads are represented as empty strings
///
/// # Wire Format
///
/// In the protocol, payloads are always transmitted as strings:
/// ```json
/// {
///     "payload": "base64_encoded_data"
/// }
/// ```
///
/// # Encoding Patterns
///
/// JSON Payload:
/// 1. Serialize payload to JSON
/// 2. Base64 encode the JSON string
///
/// Protocol Buffer Payload:
/// 1. Serialize to Protocol Buffer bytes
/// 2. DEFLATE compress the bytes
/// 3. Base64 encode the compressed data
///
/// # Examples
///
/// Playback progress payload:
/// ```rust
/// let payload = Payload::PlaybackProgress {
///     queue_id: "queue123".to_string(),
///     element_id: queue_item,
///     duration: Duration::from_secs(180),
///     buffered: Duration::from_secs(10),
///     progress: Some(Percentage::from_ratio_f32(0.5)),
///     volume: Percentage::from_ratio_f32(1.0),
///     quality: AudioQuality::Standard,
///     is_playing: true,
///     is_shuffle: false,
///     repeat_mode: RepeatMode::None,
/// };
/// ```
///
/// Queue publication payload (Protocol Buffer):
/// ```rust
/// let payload = Payload::PublishQueue(queue::List {
///     id: "queue123".to_string(),
///     // ... other queue fields ...
/// });
/// ```
///
/// Empty payload:
/// ```rust
/// let payload = Payload::String(None);
/// ```
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
// `serde_with::serde_as` seems to ignore the `rename_all` pragma together with
// `untagged`, so `rename_all` is repeated for every variant.
#[serde(untagged)]
pub enum Payload {
    /// Playback progress information.
    ///
    /// Contains current playback state including:
    /// * Track position and buffering
    /// * Volume and quality settings
    /// * Shuffle and repeat modes
    #[serde(rename_all = "camelCase")]
    PlaybackProgress {
        /// Current queue identifier
        queue_id: String,
        /// Currently playing track
        element_id: QueueItem,
        /// Total track duration
        #[serde_as(as = "Option<DurationSeconds<u64, Flexible>>")]
        duration: Option<Duration>,
        /// Amount of audio buffered
        #[serde_as(as = "Option<DurationSeconds<u64, Flexible>>")]
        buffered: Option<Duration>,
        /// Current playback position (0.0 to 1.0)
        progress: Option<Percentage>,
        /// Current volume level (0.0 to 1.0)
        volume: Percentage,
        /// Audio quality of the current track
        quality: AudioQuality,
        /// Whether playback is active
        is_playing: bool,
        /// Whether shuffle mode is enabled
        is_shuffle: bool,
        /// Current repeat mode setting
        repeat_mode: RepeatMode,
    },

    /// Message acknowledgment data.
    ///
    /// References the message being acknowledged.
    #[serde(rename_all = "camelCase")]
    Acknowledgement {
        /// ID of the acknowledged message
        acknowledgement_id: String,
    },

    /// Command execution status.
    ///
    /// Reports success or failure of a command.
    #[serde(rename_all = "camelCase")]
    Status {
        /// ID of the command being reported
        command_id: String,
        /// Execution result
        status: Status,
    },

    /// Device communication parameters.
    ///
    /// Used for device discovery and connection.
    WithParams {
        /// Source device identifier
        from: DeviceId,
        /// Connection or discovery parameters
        params: Params,
    },

    /// Playback control commands.
    ///
    /// Controls various aspects of playback state.
    #[serde(rename_all = "camelCase")]
    Skip {
        /// Target queue identifier
        queue_id: Option<String>,
        /// Track to skip to
        element_id: Option<QueueItem>,
        /// Position to seek to (0.0 to 1.0)
        progress: Option<Percentage>,
        /// Whether to start playing
        should_play: Option<bool>,
        /// New repeat mode setting
        set_repeat_mode: Option<RepeatMode>,
        /// New shuffle mode setting
        set_shuffle: Option<bool>,
        /// New volume level (0.0 to 1.0)
        set_volume: Option<Percentage>,
    },

    /// Simple string payload.
    ///
    /// Used for messages that don't require structured data.
    /// `None` represents an empty payload.
    String(#[serde_as(as = "NoneAsEmptyString")] Option<String>),

    /// Queue data in Protocol Buffer format.
    ///
    /// This variant is handled specially during serialization:
    /// * Skipped during JSON serialization
    /// * Manually encoded as compressed Protocol Buffer
    // This protobuf is deserialized manually with `FromStr`.
    #[serde(skip)]
    PublishQueue(queue::List),
}

/// Connection and discovery parameters in the Deezer Connect protocol.
///
/// These parameters are used during device discovery and connection setup
/// to exchange capabilities and session information between devices.
///
/// # Wire Format
///
/// Parameters are serialized as JSON objects with different fields
/// depending on the variant:
///
/// Connection Offer:
/// ```json
/// {
///     "deviceName": "My Phone",
///     "deviceType": "web",
///     "supportedControlVersions": ["1.0.0-beta2"]
/// }
/// ```
///
/// Discovery Request:
/// ```json
/// {
///     "discoverySession": "session-123"
/// }
/// ```
///
/// Connect Request:
/// ```json
/// {
///     "offerId": "offer-123"  // Optional
/// }
/// ```
///
/// # Examples
///
/// Creating connection offer parameters:
/// ```rust
/// let params = Params::ConnectionOffer {
///     device_name: "My Device".to_string(),
///     device_type: "web".to_string(),
///     supported_control_versions: {
///         let mut versions = HashSet::new();
///         versions.insert("1.0.0-beta2".to_string());
///         versions
///     },
/// };
/// ```
///
/// Creating discovery request parameters:
/// ```rust
/// let params = Params::DiscoveryRequest {
///     discovery_session: "session-123".to_string(),
/// };
/// ```
///
/// Creating connect request parameters:
/// ```rust
/// let params = Params::Connect {
///     offer_id: Some("offer-123".to_string()),
/// };
///
/// // Or without a specific offer
/// let params = Params::Connect {
///     offer_id: None,
/// };
/// ```
///
/// Using in messages:
/// ```rust
/// let body = Body::ConnectionOffer {
///     message_id: "msg123".to_string(),
///     from: DeviceId::default(),
///     device_name: "My Device".to_string(),
/// };
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", untagged)]
pub enum Params {
    /// Parameters for offering a connection to other devices.
    ///
    /// Used when a device announces itself for connection.
    ConnectionOffer {
        /// Human-readable device name.
        ///
        /// This name may be displayed to users in device selection UI.
        device_name: String,

        /// Type of device offering connection.
        ///
        /// Currently "web" is the only known type.
        device_type: DeviceType,

        /// Set of supported protocol versions.
        ///
        /// Used to ensure protocol compatibility between devices.
        /// Currently only "1.0.0-beta2" is supported.
        supported_control_versions: HashSet<String>,
    },

    /// Parameters for device discovery requests.
    ///
    /// Used to initiate or join a discovery session.
    DiscoveryRequest {
        /// Unique identifier for this discovery session.
        ///
        /// Multiple devices can participate in the same discovery
        /// session by using the same identifier.
        discovery_session: String,
    },

    /// Parameters for connection requests.
    ///
    /// Used when responding to a connection offer.
    Connect {
        /// Optional identifier of the offer being accepted.
        ///
        /// When None, represents an unprompted connection attempt.
        offer_id: Option<String>,
    },
}

/// Type of device offering a Deezer Connect connection.
///
/// This type is used during device discovery to identify the nature of the connecting
/// device, which can affect how it appears in device selection UIs and how other
/// devices interact with it.
///
/// # Examples
///
/// ```rust
/// use serde_json;
///
/// // Creating device types
/// let web = DeviceType::Web;  // Default, used by desktop apps
/// let mobile = DeviceType::Mobile;  // Used by smartphone apps
///
/// // Parsing from strings
/// assert_eq!("web".parse::<DeviceType>()?, DeviceType::Web);
/// assert_eq!("mobile".parse::<DeviceType>()?, DeviceType::Mobile);
///
/// // Wire format serialization
/// assert_eq!(serde_json::to_string(&DeviceType::Web)?, r#""web""#);
/// assert_eq!(serde_json::to_string(&DeviceType::Mobile)?, r#""mobile""#);
/// ```
#[derive(
    Copy,
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Debug,
    SerializeDisplay,
    DeserializeFromStr,
    Hash,
)]
pub enum DeviceType {
    /// Desktop device type
    ///
    /// Note: The official Deezer desktop applications are Electron-based and identify as `Web`.
    Desktop,

    /// Mobile Deezer client (e.g., smartphone app)
    Mobile,

    /// Tablet Deezer client (e.g., iPad app)
    Tablet,

    /// Web-based Deezer client (e.g., browser player, desktop app)
    ///
    /// This is the default variant. Desktop applications are Electron-based and use this type.
    #[default]
    Web,

    /// Unknown device type
    ///
    /// This variant catches any device types not explicitly supported,
    /// allowing forward compatibility with new device types.
    Unknown,
}

/// Formats the device type as a lowercase string for the wire protocol.
///
/// # Examples
/// ```rust
/// assert_eq!(DeviceType::Desktop.to_string(), "desktop");
/// assert_eq!(DeviceType::Mobile.to_string(), "mobile");
/// assert_eq!(DeviceType::Tablet.to_string(), "tablet");
/// assert_eq!(DeviceType::Web.to_string(), "web");
/// assert_eq!(DeviceType::Unknown.to_string(), "unknown");
/// ```
impl fmt::Display for DeviceType {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceType::Desktop => write!(f, "desktop"),
            DeviceType::Mobile => write!(f, "mobile"),
            DeviceType::Tablet => write!(f, "tablet"),
            DeviceType::Web => write!(f, "web"),
            DeviceType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Parses a device type from a string, case-insensitively.
///
/// Any unrecognized device type is parsed as `DeviceType::Unknown`,
/// providing forward compatibility with new device types.
///
/// # Examples
/// ```rust
/// use std::str::FromStr;
///
/// assert_eq!(DeviceType::from_str("desktop")?, DeviceType::Desktop);
/// assert_eq!(DeviceType::from_str("MOBILE")?, DeviceType::Mobile);
/// assert_eq!(DeviceType::from_str("unknown")?, DeviceType::Unknown);
/// assert_eq!(DeviceType::from_str("future_device")?, DeviceType::Unknown);
/// ```
impl FromStr for DeviceType {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "desktop" => Ok(DeviceType::Desktop),
            "mobile" => Ok(DeviceType::Mobile),
            "tablet" => Ok(DeviceType::Tablet),
            "web" => Ok(DeviceType::Web),
            _ => Ok(DeviceType::Unknown),
        }
    }
}

/// Formats the payload for wire transmission.
///
/// The output format depends on the payload type:
/// * `PublishQueue` - DEFLATE-compressed Protocol Buffer, Base64-encoded
/// * Other variants - JSON string, Base64-encoded
/// * Empty payloads - Empty string
///
/// # Examples
///
/// ```rust
/// // Empty payload
/// let payload = Payload::String(None);
/// assert_eq!(payload.to_string(), "");
///
/// // JSON payload
/// let payload = Payload::Status {
///     command_id: "cmd123".to_string(),
///     status: Status::OK,
/// };
/// // Results in Base64-encoded JSON
/// ```
///
/// # Errors
///
/// Returns a formatting error if:
/// * JSON serialization fails
/// * Protocol Buffer serialization fails
/// * DEFLATE compression fails
impl fmt::Display for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buffer: Vec<u8> = vec![];

        if let Payload::PublishQueue(queue) = self {
            match queue.write_to_bytes() {
                Ok(protobuf) => {
                    let mut deflater = DeflateEncoder::new(&protobuf[..], Compression::fast());
                    if let Err(e) = deflater.read_to_end(&mut buffer) {
                        error!("{e}");
                        return Err(fmt::Error);
                    }
                }
                Err(e) => {
                    error!("{e}");
                    return Err(fmt::Error);
                }
            }
        } else {
            // Do not Base64 encode empty strings.
            if let Payload::String(s) = self
                && s.as_ref().is_none_or(String::is_empty) {
                    return Ok(());
                }

            if let Err(e) = serde_json::to_writer(&mut buffer, self) {
                error!("{e}");
                return Err(fmt::Error);
            }
        }

        write!(f, "{}", BASE64_STANDARD.encode(buffer))
    }
}

/// Parses a wire format payload string into a `Payload`.
///
/// This implementation handles the various payload encoding formats:
/// * Base64-encoded JSON for most messages
/// * Base64-encoded, DEFLATE-compressed Protocol Buffers for queue data
/// * Empty strings for payloads without data
///
/// # Format Detection
///
/// The parser attempts to determine the correct format:
/// 1. If empty or "{}" → `Payload::String(None)`
/// 2. If valid UTF-8 after Base64 decode → Parse as JSON
/// 3. Otherwise → Try to parse as compressed Protocol Buffer
///
/// # Examples
///
/// Empty payload:
/// ```rust
/// let payload: Payload = "".parse()?;
/// assert!(matches!(payload, Payload::String(None)));
///
/// let payload: Payload = "{}".parse()?;
/// assert!(matches!(payload, Payload::String(None)));
/// ```
///
/// JSON payload:
/// ```rust
/// // Base64-encoded JSON: {"commandId":"cmd123","status":0}
/// let encoded = "eyJjb21tYW5kSWQiOiJjbWQxMjMiLCJzdGF0dXMiOjB9";
/// let payload: Payload = encoded.parse()?;
/// assert!(matches!(payload, Payload::Status { .. }));
/// ```
///
/// Protocol Buffer payload:
/// ```rust
/// // Base64-encoded, DEFLATE-compressed Protocol Buffer
/// let payload: Payload = compressed_base64.parse()?;
/// assert!(matches!(payload, Payload::PublishQueue(_)));
/// ```
///
/// # Errors
///
/// Returns an error if:
/// * Base64 decoding fails
/// * JSON parsing fails for JSON payloads
/// * Protocol Buffer parsing fails for queue data
/// * Compressed data cannot be decompressed
/// * Data format doesn't match any known payload type
///
/// # Notes
///
/// The implementation includes special handling for queue data to maintain
/// compatibility with different Deezer Connect clients:
/// * iOS and Android clients may send queue data differently
/// * Queue order may be explicit or implicit
/// * Shuffled queues require special handling (currently logged but not processed)
impl FromStr for Payload {
    type Err = Error;

    // TODO : first decode base64 in fromstr, then deserialize json with traits
    fn from_str(encoded: &str) -> std::result::Result<Self, Self::Err> {
        let decoded = BASE64_STANDARD.decode(encoded)?;

        if let Ok(s) = std::str::from_utf8(&decoded) {
            // Handle empty payloads
            if s.is_empty() || s == "{}" {
                return Ok(Self::String(None));
            }

            // Try parsing as JSON payload
            serde_json::from_str::<Self>(s).map_err(Into::into)
        } else {
            // Try parsing as compressed Protocol Buffer
            let mut inflater = DeflateDecoder::new(&decoded[..]);
            let mut buffer: Vec<u8> = vec![];
            inflater.read_to_end(&mut buffer)?;

            if let Ok(queue) = queue::List::parse_from_bytes(&buffer) {
                // Validate queue data by checking for required fields
                if !queue.id.is_empty() {
                    return Ok(Self::PublishQueue(queue));
                }
            }

            Err(Self::Err::unimplemented(
                "protobuf should match some variant".to_string(),
            ))
        }
    }
}

impl WireBody {
    /// Protocol version for playback control messages.
    const COMMAND_VERSION: &'static str = "com.deezer.remote.command.proto1";

    /// Protocol version for device discovery messages.
    const DISCOVERY_VERSION: &'static str = "com.deezer.remote.discovery.proto1";

    /// Protocol version for queue management messages.
    const QUEUE_VERSION: &'static str = "com.deezer.remote.queue.proto1";

    /// Supported control protocol versions.
    ///
    /// Used in device discovery to ensure compatibility.
    const SUPPORTED_CONTROL_VERSIONS: [&'static str; 1] = ["1.0.0-beta2"];

    /// Checks if a set of control versions is supported.
    ///
    /// Used during device discovery to validate compatibility.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut versions = HashSet::new();
    /// versions.insert("1.0.0-beta2".to_string());
    /// assert!(WireBody::supports_control_versions(&versions));
    ///
    /// versions.clear();
    /// versions.insert("2.0.0".to_string());
    /// assert!(!WireBody::supports_control_versions(&versions));
    /// ```
    #[must_use]
    fn supports_control_versions(control_versions: &HashSet<String>) -> bool {
        for version in control_versions {
            if Self::SUPPORTED_CONTROL_VERSIONS.contains(&version.as_str()) {
                return true;
            }
        }

        false
    }

    /// Checks if this message uses a supported protocol version.
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert!(wire_body.supported_protocol_version());  // Using COMMAND_VERSION
    /// ```
    #[must_use]
    fn supported_protocol_version(&self) -> bool {
        matches!(
            self.protocol_version.as_ref(),
            WireBody::COMMAND_VERSION | WireBody::DISCOVERY_VERSION | WireBody::QUEUE_VERSION
        )
    }
}

/// Converts a high-level [`Body`] into its wire format representation.
///
/// This conversion handles:
/// * Protocol version selection
/// * Payload encoding
/// * Message type mapping
///
/// # Examples
///
/// ```rust
/// let body = Body::Ping {
///     message_id: "msg123".to_string(),
/// };
/// let wire_body = WireBody::from(body);
///
/// assert_eq!(wire_body.message_type, MessageType::Ping);
/// assert_eq!(wire_body.protocol_version, WireBody::COMMAND_VERSION);
/// ```
impl From<Body> for WireBody {
    #[expect(clippy::too_many_lines)]
    fn from(body: Body) -> Self {
        let clock: HashMap<String, serde_json::Value> = HashMap::new();

        match body {
            Body::Acknowledgement {
                message_id,
                acknowledgement_id,
            } => WireBody {
                message_id,
                message_type: MessageType::Acknowledgement,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::Acknowledgement { acknowledgement_id },
                clock,
            },

            Body::Close { message_id } => WireBody {
                message_id,
                message_type: MessageType::Close,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::String(None),
                clock,
            },

            Body::Connect {
                message_id,
                from,
                offer_id,
            } => WireBody {
                message_id,
                message_type: MessageType::Connect,
                protocol_version: Self::DISCOVERY_VERSION.to_string(),
                payload: Payload::WithParams {
                    from,
                    params: Params::Connect { offer_id },
                },
                clock,
            },

            Body::ConnectionOffer {
                message_id,
                from,
                device_name,
                device_type,
            } => WireBody {
                message_id,
                message_type: MessageType::ConnectionOffer,
                protocol_version: Self::DISCOVERY_VERSION.to_string(),
                payload: Payload::WithParams {
                    from,
                    params: Params::ConnectionOffer {
                        device_name,
                        device_type,
                        supported_control_versions: Self::SUPPORTED_CONTROL_VERSIONS
                            .into_iter()
                            .map(ToString::to_string)
                            .collect(),
                    },
                },
                clock,
            },

            Body::DiscoveryRequest {
                message_id,
                from,
                discovery_session,
            } => WireBody {
                message_id,
                message_type: MessageType::DiscoveryRequest,
                protocol_version: Self::DISCOVERY_VERSION.to_string(),
                payload: Payload::WithParams {
                    from,
                    params: Params::DiscoveryRequest { discovery_session },
                },
                clock,
            },

            Body::Ping { message_id } => WireBody {
                message_id,
                message_type: MessageType::Ping,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::String(None),
                clock,
            },

            Body::PlaybackProgress {
                message_id,
                track,
                duration,
                buffered,
                progress,
                volume,
                quality,
                is_playing,
                is_shuffle,
                repeat_mode,
            } => WireBody {
                message_id,
                message_type: MessageType::PlaybackProgress,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::PlaybackProgress {
                    queue_id: track.queue_id.clone(),
                    element_id: track,
                    duration,
                    buffered,
                    progress,
                    volume,
                    quality,
                    is_playing,
                    is_shuffle,
                    repeat_mode,
                },
                clock,
            },

            Body::PublishQueue { message_id, queue } => WireBody {
                message_id,
                message_type: MessageType::PublishQueue,
                protocol_version: Self::QUEUE_VERSION.to_string(),
                payload: Payload::PublishQueue(queue),
                clock,
            },

            Body::Ready { message_id } => WireBody {
                message_id,
                message_type: MessageType::Ready,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::String(None),
                clock,
            },

            Body::RefreshQueue { message_id } => WireBody {
                message_id,
                message_type: MessageType::RefreshQueue,
                protocol_version: Self::QUEUE_VERSION.to_string(),
                payload: Payload::String(None),
                clock,
            },

            Body::Skip {
                message_id,
                queue_id,
                track,
                progress,
                should_play,
                set_shuffle,
                set_repeat_mode,
                set_volume,
            } => WireBody {
                message_id,
                message_type: MessageType::Skip,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::Skip {
                    queue_id,
                    element_id: track,
                    progress,
                    should_play,
                    set_shuffle,
                    set_repeat_mode,
                    set_volume,
                },
                clock,
            },

            Body::Status {
                message_id,
                command_id,
                status,
            } => WireBody {
                message_id,
                message_type: MessageType::Status,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::Status { command_id, status },
                clock,
            },

            Body::Stop { message_id } => WireBody {
                message_id,
                message_type: MessageType::Stop,
                protocol_version: Self::COMMAND_VERSION.to_string(),
                payload: Payload::String(None),
                clock,
            },
        }
    }
}

/// Attempts to convert a wire format message into a high-level [`Body`].
///
/// This conversion handles:
/// * Protocol version validation
/// * Payload decoding
/// * Message type verification
/// * Data structure validation
///
/// # Protocol Version Handling
///
/// While unknown protocol versions generate a warning, they don't cause
/// conversion failure. This allows for forward compatibility with newer
/// protocol versions.
///
/// # Examples
///
/// Success case:
/// ```rust
/// let wire_body = WireBody {
///     message_id: "msg123".to_string(),
///     message_type: MessageType::Ping,
///     protocol_version: WireBody::COMMAND_VERSION.to_string(),
///     payload: Payload::String(None),
///     clock: HashMap::new(),
/// };
///
/// let body = Body::try_from(wire_body)?;
/// assert!(matches!(body, Body::Ping { .. }));
/// ```
///
/// Protocol version warning:
/// ```rust
/// let wire_body = WireBody {
///     protocol_version: "com.deezer.remote.command.proto2".to_string(),
///     // ... other fields ...
/// };
///
/// // Conversion still succeeds but logs a warning
/// let body = Body::try_from(wire_body)?;
/// ```
///
/// Payload mismatch:
/// ```rust
/// let wire_body = WireBody {
///     message_type: MessageType::Ping,
///     payload: Payload::PlaybackProgress { .. },  // Wrong payload type
///     // ... other fields ...
/// };
///
/// assert!(Body::try_from(wire_body).is_err());
/// ```
///
/// # Errors
///
/// Returns an error if:
/// * Message payload doesn't match its declared type
/// * Required payload fields are missing
/// * Payload data is malformed
/// * Message type is unknown or unsupported
///
/// # Notes
///
/// The conversion is intentionally permissive with protocol versions to
/// maintain compatibility with future protocol updates. However, it's
/// strict about payload structure to ensure data integrity.
impl TryFrom<WireBody> for Body {
    type Error = Error;

    #[expect(clippy::too_many_lines)]
    fn try_from(wire_body: WireBody) -> std::result::Result<Self, Self::Error> {
        if !wire_body.supported_protocol_version() {
            warn!("protocol version {} is unknown", wire_body.protocol_version);
        }

        let message_id = wire_body.message_id;
        let message_type = wire_body.message_type;

        let body = match message_type {
            MessageType::Acknowledgement => {
                if let Payload::Acknowledgement { acknowledgement_id } = wire_body.payload {
                    Self::Acknowledgement {
                        message_id,
                        acknowledgement_id,
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::Close => Body::Close { message_id },

            MessageType::Connect => {
                if let Payload::WithParams { from, params } = wire_body.payload {
                    if let Params::Connect { offer_id } = params {
                        Self::Connect {
                            message_id,
                            from,
                            offer_id,
                        }
                    } else {
                        trace!("{params:#?}");
                        return Err(Self::Error::failed_precondition(format!(
                            "params should match message type {message_type}"
                        )));
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::ConnectionOffer => {
                if let Payload::WithParams { from, params } = wire_body.payload {
                    if let Params::ConnectionOffer {
                        device_name,
                        device_type,
                        supported_control_versions,
                        ..
                    } = params
                    {
                        if !WireBody::supports_control_versions(&supported_control_versions) {
                            warn!("control versions {supported_control_versions:?} are unknown");
                        }

                        Self::ConnectionOffer {
                            message_id,
                            from,
                            device_name,
                            device_type,
                        }
                    } else {
                        trace!("{params:#?}");
                        return Err(Self::Error::failed_precondition(format!(
                            "params should match message type {message_type}"
                        )));
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::DiscoveryRequest => {
                if let Payload::WithParams { from, params } = wire_body.payload {
                    if let Params::DiscoveryRequest { discovery_session } = params {
                        Self::DiscoveryRequest {
                            message_id,
                            from,
                            discovery_session,
                        }
                    } else {
                        trace!("{params:#?}");
                        return Err(Self::Error::failed_precondition(format!(
                            "params should match message type {message_type}"
                        )));
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::Ping => Self::Ping { message_id },

            MessageType::PlaybackProgress => {
                if let Payload::PlaybackProgress {
                    element_id,
                    buffered,
                    duration,
                    progress,
                    volume,
                    quality,
                    is_playing,
                    is_shuffle,
                    repeat_mode,
                    ..
                } = wire_body.payload
                {
                    Self::PlaybackProgress {
                        message_id,
                        track: element_id,
                        buffered,
                        duration,
                        progress,
                        volume,
                        quality,
                        is_playing,
                        is_shuffle,
                        repeat_mode,
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::PublishQueue => {
                if let Payload::PublishQueue(queue) = wire_body.payload {
                    Self::PublishQueue { message_id, queue }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::Ready => Self::Ready { message_id },

            MessageType::RefreshQueue => Self::RefreshQueue { message_id },

            MessageType::Skip => {
                if let Payload::Skip {
                    queue_id,
                    element_id,
                    progress,
                    should_play,
                    set_shuffle,
                    set_repeat_mode,
                    set_volume,
                    ..
                } = wire_body.payload
                {
                    Self::Skip {
                        message_id,
                        queue_id,
                        track: element_id,
                        progress,
                        should_play,
                        set_shuffle,
                        set_repeat_mode,
                        set_volume,
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::Status => {
                if let Payload::Status { command_id, status } = wire_body.payload {
                    Self::Status {
                        message_id,
                        command_id,
                        status,
                    }
                } else {
                    trace!("{:#?}", wire_body.payload);
                    return Err(Self::Error::failed_precondition(format!(
                        "payload should match message type {message_type}"
                    )));
                }
            }

            MessageType::Stop => Self::Stop { message_id },
        };

        Ok(body)
    }
}

/// Formats the message type as its variant name.
///
/// This implementation is primarily used for logging and debugging.
/// For wire format serialization, use the `Serialize` implementation.
///
/// # Examples
///
/// ```rust
/// assert_eq!(MessageType::Ping.to_string(), "Ping");
/// assert_eq!(MessageType::PlaybackProgress.to_string(), "PlaybackProgress");
///
/// // Useful for logging
/// println!("Received message type: {}", MessageType::ConnectionOffer);
/// ```
impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
