//! Message types and routing for the Deezer Connect protocol.
//!
//! This module provides a high-level message representation for communication over [`Channel`]s.
//!
//! Messages flow through channels in specific directions:
//! * `Send`/`Receive` - Regular message content
//! * `StreamSend`/`StreamReceive` - Playback reporting
//! * `Subscribe`/`Unsubscribe` - Channel management
//!
//! # Message Structure
//!
//! Messages in the Deezer Connect protocol follow a specific structure:
//! ```json
//! [
//!     "<stanza>",          // Message type (send/receive/sub/unsub)
//!     "<channel>",         // Channel identifier
//!     { "payload": ... }   // Optional payload (for content messages)
//! ]
//! ```
//!
//! # Usage
//!
//! Application code should use the [`Message`] type, which provides a
//! strongly-typed interface:
//!
//! ```rust
//! use deezer::{Channel, Contents, Ident, Message};
//!
//! // Create a message to send
//! let msg = Message::Send {
//!     channel: Channel::new(Ident::RemoteCommand),
//!     contents: Contents { /* ... */ },
//! };
//!
//! // Create a subscription
//! let sub = Message::Subscribe {
//!     channel: Channel::new(Ident::RemoteCommand),
//! };
//! ```
//!
//! The lower-level wire format types are handled automatically during
//! serialization/deserialization.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{Channel, Contents, stream};
use crate::error::Error;

/// Primary message type for the Deezer Connect protocol.
///
/// This enum provides a strongly-typed interface for messages exchanged over Deezer Connect
/// websockets, focusing specifically on remote control functionality. It deliberately omits
/// auxiliary protocol messages (like UX tracking) that aren't relevant to remote control.
///
/// # Message Categories
///
/// Messages fall into three categories:
/// * Regular content messages ([`Send`](Self::Send)/[`Receive`](Self::Receive))
/// * Stream reporting messages ([`StreamSend`](Self::StreamSend)/[`StreamReceive`](Self::StreamReceive))
/// * Channel subscriptions ([`Subscribe`](Self::Subscribe)/[`Unsubscribe`](Self::Unsubscribe))
///
/// # Wire Format
///
/// While this type provides a Rust-friendly interface, messages are serialized to JSON arrays:
/// ```json
/// ["send", "channel_name", {"payload": "data"}]  // Content message
/// ["sub", "channel_name"]                        // Subscription
/// ```
///
/// # Examples
///
/// ```rust
/// use deezer::{Channel, Contents, Ident, Message};
///
/// // Sending content
/// let msg = Message::Send {
///     channel: Channel::new(Ident::RemoteCommand),
///     contents: Contents { /* ... */ },
/// };
///
/// // Managing subscriptions
/// let msg = Message::Subscribe {
///     channel: Channel::new(Ident::RemoteCommand),
/// };
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Send content to a channel.
    ///
    /// This variant represents an outgoing message carrying [`Contents`] data
    /// to be sent over a [`Channel`].
    Send {
        /// Target channel for the message
        channel: Channel,
        /// Content data to send
        contents: Contents,
    },

    /// Receive content from a channel.
    ///
    /// This variant represents an incoming message carrying [`Contents`] data
    /// received from a [`Channel`].
    Receive {
        /// Channel that received the message
        channel: Channel,
        /// Content data received
        contents: Contents,
    },

    /// Send playback stream report to a channel.
    ///
    /// This variant represents an outgoing message reporting [`stream::Contents`]
    /// status to a [`Channel`]. Used to inform other devices about active
    /// playback streams.
    StreamSend {
        /// Target channel for the report
        channel: Channel,
        /// Stream status to report
        contents: stream::Contents,
    },

    /// Receive playback stream report from a channel.
    ///
    /// This variant represents an incoming message containing [`stream::Contents`]
    /// status from a [`Channel`]. Used to receive information about other
    /// devices' active playback streams.
    StreamReceive {
        /// Channel that received the report
        channel: Channel,
        /// Stream status received
        contents: stream::Contents,
    },

    /// Subscribe to a channel.
    ///
    /// This variant represents a request to start receiving messages from
    /// the specified [`Channel`].
    Subscribe {
        /// Channel to subscribe to
        channel: Channel,
    },

    /// Unsubscribe from a channel.
    ///
    /// This variant represents a request to stop receiving messages from
    /// the specified [`Channel`].
    Unsubscribe {
        /// Channel to unsubscribe from
        channel: Channel,
    },
}

/// Formats a message for display, showing direction and contents.
///
/// The output format depends on the message type:
/// * Content/Stream messages: `"{channel} {direction} {contents}"`
/// * Subscriptions: `"subscribing to {channel}"`/`"unsubscribing from {channel}"`
///
/// The channel identifier is padded to 14 characters for alignment.
///
/// # Examples
///
/// ```rust
/// let msg = Message::Send { /* ... */ };
/// // Prints: "RemoteCommand  -> PlaybackProgress"
/// println!("{msg}");
///
/// let msg = Message::Subscribe { /* ... */ };
/// // Prints: "subscribing to RemoteCommand"
/// println!("{msg}");
/// ```
///
/// # Notes
///
/// Currently has a known limitation where padding is not respected.
impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // FIXME: padding is not respected.
        match self {
            Self::Send { channel, contents } => {
                write!(f, "{:<14} -> {contents}", channel.ident)
            }
            Self::Receive { channel, contents } => {
                write!(f, "{:<14} <- {contents}", channel.ident)
            }
            Self::StreamSend { channel, contents } => {
                write!(f, "{:<14} -> {contents}", channel.ident)
            }
            Self::StreamReceive { channel, contents } => {
                write!(f, "{:<14} <- {contents}", channel.ident)
            }
            Self::Subscribe { channel } => write!(f, "subscribing to {channel}"),
            Self::Unsubscribe { channel } => write!(f, "unsubscribing from {channel}"),
        }
    }
}

/// Serializes a message into its wire format representation.
///
/// # Examples
///
/// ```rust
/// let msg = Message::Send { /* ... */ };
/// let json = serde_json::to_string(&msg)?;
/// // Results in: ["send", "channel", {...}]
/// ```
///
/// # Errors
///
/// Returns an error if:
/// * Channel and content identifiers don't match
/// * JSON serialization fails
impl Serialize for Message {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire_message =
            WireMessage::try_from(self.clone()).map_err(serde::ser::Error::custom)?;
        wire_message.serialize(serializer)
    }
}

/// Deserializes a message from its wire format representation.
///
/// # Examples
///
/// ```rust
/// // Wire format: ["send", "channel", {...}]
/// let msg: Message = serde_json::from_str(json)?;
/// ```
///
/// # Errors
///
/// Returns an error if:
/// * JSON is malformed
/// * Channel and content identifiers don't match
/// * Message format is invalid
impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire_message = WireMessage::deserialize(deserializer)?;
        Self::try_from(wire_message).map_err(serde::de::Error::custom)
    }
}

/// Internal wire format representation of Deezer Connect protocol messages.
///
/// This type represents messages in their raw JSON array format, serving as an
/// intermediate representation between [`Message`] and the wire protocol.
///
/// # Wire Format Rules
///
/// Messages must follow these rules:
///
/// Content messages (3 elements):
/// ```json
/// [
///     "<stanza>",          // Must be "msg" or "send"
///     "<channel>",         // Must be valid channel string
///     {                    // Must be valid Contents or stream::Contents
///         "payload": {}    // Message-specific payload
///     }
/// ]
/// ```
///
/// Subscription messages (2 elements):
/// ```json
/// [
///     "<stanza>",    // Must be "sub" or "unsub"
///     "<channel>"    // Must be valid channel string
/// ]
/// ```
///
/// # Validation Rules
///
/// The following is enforced during parsing:
/// * Array must have exactly 2 or 3 elements
/// * First element (stanza) must match message type:
///   - "msg"/"send" for content messages
///   - "sub"/"unsub" for subscriptions
/// * Channel identifiers must match between channel and contents
/// * Content messages must have valid JSON payloads
///
/// # Examples
///
/// Valid messages:
/// ```rust
/// use serde_json::json;
///
/// // Content message
/// let valid = json!([
///     "send",
///     "12345_-1_REMOTECOMMAND",
///     {
///         "APP": "REMOTECOMMAND",
///         "headers": {
///             "from": "device-123",
///             "destination": null
///         },
///         "body": {
///             "messageId": "msg-123",
///             "messageType": "ping"
///         }
///     }
/// ]);
/// assert!(serde_json::from_value::<WireMessage>(valid).is_ok());
///
/// // Subscription
/// let valid = json!([
///     "sub",
///     "12345_-1_REMOTECOMMAND"
/// ]);
/// assert!(serde_json::from_value::<WireMessage>(valid).is_ok());
/// ```
///
/// Error cases:
/// ```rust
/// use serde_json::json;
///
/// // Wrong number of elements
/// let invalid = json!(["send", "channel"]);
/// assert!(serde_json::from_value::<WireMessage>(invalid).is_err());
///
/// // Mismatched stanza and content
/// let invalid = json!([
///     "sub",  // Subscription stanza
///     "12345_-1_REMOTECOMMAND",
///     { "payload": {} }  // Should not have content
/// ]);
/// assert!(serde_json::from_value::<WireMessage>(invalid).is_err());
///
/// // Invalid channel format
/// let invalid = json!([
///     "send",
///     "invalid_channel",  // Missing components
///     { "payload": {} }
/// ]);
/// assert!(serde_json::from_value::<WireMessage>(invalid).is_err());
///
/// // Mismatched identifiers
/// let invalid = json!([
///     "send",
///     "12345_-1_REMOTECOMMAND",
///     {
///         "APP": "STREAM",  // Should match channel
///         "headers": {},
///         "body": {}
///     }
/// ]);
/// assert!(serde_json::from_value::<WireMessage>(invalid).is_err());
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
enum WireMessage {
    /// Content message sequence.
    ///
    /// Represents a three-element array containing:
    /// 1. Stanza (`"msg"` or `"send"`)
    /// 2. Channel identifier
    /// 3. Message contents
    WithContents(Stanza, Channel, Contents),

    /// Stream report message sequence.
    ///
    /// Represents a three-element array containing:
    /// 1. Stanza (`"msg"` or `"send"`)
    /// 2. Channel identifier
    /// 3. Stream status contents
    WithStreamContents(Stanza, Channel, stream::Contents),

    /// Subscription message sequence.
    ///
    /// Represents a two-element array containing:
    /// 1. Stanza (`"sub"` or `"unsub"`)
    /// 2. Channel identifier
    ///
    // Note: This variant must be last to prevent it from matching three-element arrays.
    Subscription(Stanza, Channel),
}

/// Message type indicator in the Deezer Connect protocol.
///
/// A stanza is the first element in a message array and indicates how to interpret
/// the message. It determines both the message's direction (incoming/outgoing) and
/// its purpose (content/subscription).
///
/// # Wire Format
///
/// Stanzas are serialized as specific strings:
/// * `"msg"` - [`Receive`](Self::Receive) (incoming message)
/// * `"send"` - [`Send`](Self::Send) (outgoing message)
/// * `"sub"` - [`Subscribe`](Self::Subscribe) (channel subscription)
/// * `"unsub"` - [`Unsubscribe`](Self::Unsubscribe) (channel unsubscription)
#[derive(Copy, Clone, Debug, Hash, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum Stanza {
    /// Marks an incoming message received over a channel.
    #[serde(rename = "msg")]
    Receive,

    /// Marks an outgoing message to be sent to a channel.
    #[serde(rename = "send")]
    Send,

    /// Marks a request to subscribe to a channel.
    #[serde(rename = "sub")]
    Subscribe,

    /// Marks a request to unsubscribe from a channel.
    #[serde(rename = "unsub")]
    Unsubscribe,
}

/// Formats the stanza for display using its variant name.
///
/// # Examples
///
/// ```rust
/// assert_eq!(Stanza::Receive.to_string(), "Receive");
/// assert_eq!(Stanza::Send.to_string(), "Send");
/// ```
impl fmt::Display for Stanza {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Converts a [`Message`] into its wire format representation.
///
/// # Errors
///
/// Returns an error if:
/// * Channel identifier doesn't match content identifier in content messages
///
/// # Examples
///
/// ```rust
/// let msg = Message::Send {
///     channel: Channel::new(Ident::RemoteCommand),
///     contents: Contents { /* ... */ },
/// };
/// let wire_msg = WireMessage::try_from(msg)?;
/// ```
impl TryFrom<Message> for WireMessage {
    type Error = Error;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        let wire_message = match message {
            Message::Receive { channel, contents } => {
                let contents_ident = contents.ident;
                let channel_ident = channel.ident;
                if contents_ident != channel_ident {
                    return Err(Self::Error::failed_precondition(format!(
                        "channel identifier {channel_ident} should match content identifier {contents_ident}",
                    )));
                }

                Self::WithContents(Stanza::Receive, channel, contents)
            }

            Message::Send { channel, contents } => {
                let contents_ident = contents.ident;
                let channel_ident = channel.ident;
                if contents_ident != channel_ident {
                    return Err(Self::Error::failed_precondition(format!(
                        "channel identifier {channel_ident} should match content identifier {contents_ident}",
                    )));
                }

                Self::WithContents(Stanza::Send, channel, contents)
            }

            // On `Stream` channels, the `Event` value is not equal to the `Stream` name.
            Message::StreamReceive { channel, contents } => {
                Self::WithStreamContents(Stanza::Receive, channel, contents)
            }

            Message::StreamSend { channel, contents } => {
                Self::WithStreamContents(Stanza::Send, channel, contents)
            }

            Message::Subscribe { channel } => Self::Subscription(Stanza::Subscribe, channel),
            Message::Unsubscribe { channel } => Self::Subscription(Stanza::Unsubscribe, channel),
        };

        Ok(wire_message)
    }
}

/// Converts a wire format message into a [`Message`].
///
/// # Errors
///
/// Returns an error if:
/// * Channel identifier doesn't match content identifier in content messages
/// * Stanza type doesn't match message format (e.g., `Subscribe` stanza with contents)
///
/// # Examples
///
/// ```rust
/// let wire_msg = WireMessage::WithContents(
///     Stanza::Send,
///     Channel::new(Ident::RemoteCommand),
///     Contents { /* ... */ },
/// );
/// let msg = Message::try_from(wire_msg)?;
/// ```
impl TryFrom<WireMessage> for Message {
    type Error = Error;

    fn try_from(wire_message: WireMessage) -> Result<Self, Self::Error> {
        let message = match wire_message {
            WireMessage::WithContents(stanza, channel, contents) => {
                let contents_ident = contents.ident;
                let channel_ident = channel.ident;
                if contents_ident != channel_ident {
                    return Err(Self::Error::failed_precondition(format!(
                        "channel identifier {channel_ident} should match content identifier {contents_ident}",
                    )));
                }

                match stanza {
                    Stanza::Send => Self::Send { channel, contents },
                    Stanza::Receive => Self::Receive { channel, contents },
                    _ => {
                        return Err(Self::Error::failed_precondition(format!(
                            "stanza {stanza} should match for message with contents"
                        )));
                    }
                }
            }

            // On `Stream` channels, the `Event` value is not equal to the `Stream` name.
            WireMessage::WithStreamContents(stanza, channel, contents) => match stanza {
                Stanza::Send => Self::StreamSend { channel, contents },
                Stanza::Receive => Self::StreamReceive { channel, contents },
                _ => {
                    return Err(Self::Error::failed_precondition(format!(
                        "stanza {stanza} should match for stream message with contents"
                    )));
                }
            },

            WireMessage::Subscription(stanza, channel) => match stanza {
                Stanza::Subscribe => Self::Subscribe { channel },
                Stanza::Unsubscribe => Self::Unsubscribe { channel },
                _ => {
                    return Err(Self::Error::failed_precondition(format!(
                        "stanza {stanza} should match for subscription message"
                    )));
                }
            },
        };

        Ok(message)
    }
}
