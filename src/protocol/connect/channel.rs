//! Channel and message routing types for the Deezer Connect protocol.
//!
//! This module defines the routing components used in Deezer Connect communication:
//! * `Channel` - Defines message routing between users
//! * `UserId` - Identifies Deezer users or broadcast targets
//! * `Ident` - Specifies message types and their purposes
//!
//! # Wire Format
//!
//! Channels in the protocol are represented as string triplets:
//! ```text
//! <from>_<to>_<ident>
//! ```
//! Where:
//! * `from`: Sender's user ID or `-1` for unspecified
//! * `to`: Recipient's user ID or `-1` for unspecified
//! * `ident`: Message type identifier (e.g., "REMOTECOMMAND", "STREAM")
//!
//! # Examples
//!
//! ```rust
//! use std::num::NonZeroU64;
//!
//! // Create a channel for remote commands
//! let channel = Channel {
//!     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
//!     to: UserId::Unspecified,
//!     ident: Ident::RemoteCommand,
//! };
//!
//! // Serialize to wire format
//! assert_eq!(channel.to_string(), "12345_-1_REMOTECOMMAND");
//!
//! // Parse from wire format
//! let parsed: Channel = "12345_-1_REMOTECOMMAND".parse().unwrap();
//! assert_eq!(channel, parsed);
//! ```
//!
//! # Message Types
//!
//! The protocol supports several message categories:
//! * `RemoteCommand` - Playback control and status
//! * `RemoteDiscover` - Device discovery and connection
//! * `RemoteQueue` - Queue publications and UI refresh signals
//! * `Stream` - Playback reporting for analytics and monetization
//! * `UserFeed` - Social interactions (follows, comments, shares)
//!
//! [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079

use std::{
    fmt::{self, Write},
    num::NonZeroU64,
    str::FromStr,
};

use serde::Deserialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::error::Error;

/// Represents a communication channel on a [Deezer Connect][Connect] websocket.
///
/// A `Channel` is the fundamental communication pathway in the Deezer Connect protocol,
/// consisting of three components that define the message routing and type:
///
/// * `from` - The sender's Deezer user ID
/// * `to` - The recipient's Deezer user ID
/// * `ident` - The type of message being transmitted
///
/// # Wire Format
///
/// Channels are serialized to and from a wire format using the following pattern:
/// ```text
/// <from>_<to>_<ident>
/// ```
/// Where `_` is the separator character, and each component follows its own formatting rules:
/// * `from` and `to`: Either a numeric user ID or `-1` for unspecified
/// * `ident`: An uppercase string identifier, optionally with a user ID suffix
///
/// # Examples
///
/// Creating a channel for remote command messages:
/// ```rust
/// use std::num::NonZeroU64;
///
/// let channel = Channel {
///     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
///     to: UserId::Unspecified,
///     ident: Ident::RemoteCommand,
/// };
///
/// // Serializes to: "12345_-1_REMOTECOMMAND"
/// println!("{}", channel);
/// ```
///
/// Creating a channel for user feed messages:
/// ```rust
/// use std::num::NonZeroU64;
///
/// let channel = Channel {
///     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
///     to: UserId::Id(NonZeroU64::new(67890).unwrap()),
///     ident: Ident::UserFeed(UserId::Id(NonZeroU64::new(67890).unwrap())),
/// };
///
/// // Serializes to: "12345_67890_USERFEED_67890"
/// println!("{}", channel);
/// ```
///
/// Parsing a channel from a wire format string:
/// ```rust
/// let channel: Channel = "12345_-1_REMOTECOMMAND".parse().unwrap();
/// assert_eq!(channel.from, UserId::Id(NonZeroU64::new(12345).unwrap()));
/// assert_eq!(channel.to, UserId::Unspecified);
/// assert_eq!(channel.ident, Ident::RemoteCommand);
/// ```
///
/// # Error Handling
///
/// When parsing from a string, several error conditions may occur:
/// * Invalid format (missing components or separators)
/// * Invalid user IDs (non-numeric or zero values)
/// * Unknown message identifier
/// * Invalid suffix for `UserFeed` messages
///
/// ```rust
/// // Invalid format (missing component)
/// assert!("12345_-1".parse::<Channel>().is_err());
///
/// // Invalid user ID
/// assert!("0_-1_REMOTECOMMAND".parse::<Channel>().is_err());
///
/// // Unknown message type
/// assert!("12345_-1_UNKNOWN".parse::<Channel>().is_err());
/// ```
///
/// # Serialization
///
/// The type implements [`Display`] and [`FromStr`] traits, allowing for easy
/// conversion to and from string representations. It also implements Serde's
/// [`DeserializeFromStr`] and [`SerializeDisplay`] for integration with
/// serialization frameworks.
///
/// ```rust
/// use serde_json;
///
/// let channel = Channel {
///     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
///     to: UserId::Unspecified,
///     ident: Ident::RemoteCommand,
/// };
///
/// // Serialize to JSON
/// let json = serde_json::to_string(&channel).unwrap();
/// assert_eq!(json, "\"12345_-1_REMOTECOMMAND\"");
///
/// // Deserialize from JSON
/// let deserialized: Channel = serde_json::from_str(&json).unwrap();
/// assert_eq!(channel, deserialized);
/// ```
///
/// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
/// [`Display`]: std::fmt::Display
/// [`FromStr`]: std::str::FromStr
/// [`DeserializeFromStr`]: serde_with::DeserializeFromStr
/// [`SerializeDisplay`]: serde_with::SerializeDisplay
#[derive(Copy, Clone, Debug, Hash, SerializeDisplay, DeserializeFromStr, PartialEq)]
pub struct Channel {
    /// The sending user's Deezer ID. Can be a specific user or [`UserId::Unspecified`].
    pub from: UserId,

    /// The receiving user's Deezer ID. Can be a specific user or [`UserId::Unspecified`].
    pub to: UserId,

    /// The type of message being transmitted over this channel.
    pub ident: Ident,
}

/// Represents a user identifier in the Deezer ecosystem.
///
/// A `UserId` can either be a specific user's numeric identifier or represent
/// an unspecified user (used in broadcast scenarios or when the user is unknown).
///
/// # Wire Format
///
/// In the wire protocol, user IDs are represented as:
/// * Specific users: A positive integer (e.g., "12345")
/// * Unspecified user: "-1"
///
/// # Examples
///
/// Creating specific user IDs:
/// ```rust
/// use std::num::NonZeroU64;
///
/// // From a NonZeroU64
/// let id = NonZeroU64::new(12345).unwrap();
/// let user = UserId::Id(id);
/// assert_eq!(user.to_string(), "12345");
///
/// // Using From trait
/// let user: UserId = NonZeroU64::new(12345).unwrap().into();
/// assert_eq!(user.to_string(), "12345");
/// ```
///
/// Creating an unspecified user:
/// ```rust
/// let unspec = UserId::Unspecified;
/// assert_eq!(unspec.to_string(), "-1");
/// ```
///
/// Parsing from string representations:
/// ```rust
/// // Parse a specific user
/// let user: UserId = "12345".parse().unwrap();
/// assert!(matches!(user, UserId::Id(_)));
///
/// // Parse an unspecified user
/// let unspec: UserId = "-1".parse().unwrap();
/// assert_eq!(unspec, UserId::Unspecified);
///
/// // Invalid cases
/// assert!("0".parse::<UserId>().is_err());  // Zero is invalid
/// assert!("abc".parse::<UserId>().is_err()); // Non-numeric is invalid
/// assert!("-2".parse::<UserId>().is_err());  // Only -1 is valid negative
/// ```
///
/// Using in channel routing:
/// ```rust
/// # use std::num::NonZeroU64;
/// let channel = Channel {
///     // Specific sender
///     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
///     // Broadcast to all users
///     to: UserId::Unspecified,
///     ident: Ident::RemoteCommand,
/// };
/// ```
///
/// Comparison and ordering:
/// ```rust
/// use std::num::NonZeroU64;
///
/// let user1 = UserId::Id(NonZeroU64::new(1).unwrap());
/// let user2 = UserId::Id(NonZeroU64::new(2).unwrap());
/// let unspec = UserId::Unspecified;
///
/// // Supports equality comparison
/// assert_ne!(user1, user2);
/// assert_ne!(user1, unspec);
///
/// // Supports ordering
/// assert!(user1 < user2);
/// assert!(unspec < user1); // Unspecified is always less than specific IDs
/// ```
///
/// # Error Handling
///
/// When parsing from a string, the following error conditions are handled:
/// * Non-numeric values
/// * Zero values (not allowed as valid user IDs)
/// * Negative values other than -1
/// * Values that exceed `u64::MAX`
///
/// ```rust
/// // Various error cases
/// assert!("0".parse::<UserId>().is_err());
/// assert!("abc".parse::<UserId>().is_err());
/// assert!("-2".parse::<UserId>().is_err());
/// assert!("18446744073709551616".parse::<UserId>().is_err()); // > u64::MAX
/// ```
///
/// # Serialization
///
/// The type implements [`Display`] and [`FromStr`] traits for string conversion,
/// and derives [`Hash`], [`PartialEq`], [`Eq`], [`PartialOrd`], and [`Ord`] for
/// use in collections and sorting.
///
/// ```rust
/// use std::collections::HashSet;
/// use std::num::NonZeroU64;
///
/// let mut users = HashSet::new();
/// users.insert(UserId::Id(NonZeroU64::new(12345).unwrap()));
/// users.insert(UserId::Unspecified);
///
/// assert_eq!(users.len(), 2);
/// ```
///
/// # Notes
///
/// * User IDs must be non-zero positive integers when specific
/// * The special value `-1` is reserved for [`UserId::Unspecified`]
/// * The type is optimized for efficient storage and comparison
///
/// [`Display`]: std::fmt::Display
/// [`FromStr`]: std::str::FromStr
/// [`Hash`]: std::hash::Hash
/// [`PartialEq`]: std::cmp::PartialEq
/// [`Eq`]: std::cmp::Eq
/// [`PartialOrd`]: std::cmp::PartialOrd
/// [`Ord`]: std::cmp::Ord
#[derive(Copy, Clone, Debug, Deserialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum UserId {
    /// A specific Deezer user, identified by a non-zero positive integer.
    ///
    /// This variant represents an actual user account in the Deezer system.
    Id(NonZeroU64),

    /// Represents an unspecified or broadcast user identifier.
    ///
    /// This variant is used in several contexts:
    /// * As a sender: represents messages from any user
    /// * As a receiver: represents broadcast messages to all users
    /// * In patterns: matches any user ID
    Unspecified,
}

/// Identifies the type of message being transmitted over a Deezer Connect channel.
///
/// Each variant represents a distinct message category in the Deezer Connect protocol,
/// enabling different types of communication between devices and users.
///
/// # Message Categories
///
/// * `RemoteCommand` - Playback control and status messages (play, pause, seek, etc.)
/// * `RemoteDiscover` - Device discovery and connection management
/// * `RemoteQueue` - Full playback queue publications and UI refresh signals
/// * `Stream` - Playback reporting for monetization and tracking
/// * `UserFeed` - Social interactions with user-specific targeting
///
/// # Wire Format
///
/// In the wire protocol, identifiers are represented as uppercase strings:
/// * Basic types: Simple uppercase strings (e.g., "REMOTECOMMAND")
/// * `UserFeed`: Includes user ID suffix (e.g., "`USERFEED_12345`")
///
/// # Examples
///
/// Creating basic message identifiers:
/// ```rust
/// // Remote control commands
/// let cmd = Ident::RemoteCommand;
/// assert_eq!(cmd.to_string(), "REMOTECOMMAND");
///
/// // Device discovery
/// let discover = Ident::RemoteDiscover;
/// assert_eq!(discover.to_string(), "REMOTEDISCOVER");
///
/// // Queue management
/// let queue = Ident::RemoteQueue;
/// assert_eq!(queue.to_string(), "REMOTEQUEUE");
///
/// // Stream notifications
/// let stream = Ident::Stream;
/// assert_eq!(stream.to_string(), "STREAM");
/// ```
///
/// Creating and using `UserFeed` identifiers:
/// ```rust
/// use std::num::NonZeroU64;
///
/// // Create UserFeed with specific target
/// let user_id = UserId::Id(NonZeroU64::new(12345).unwrap());
/// let feed = Ident::UserFeed(user_id);
/// assert_eq!(feed.to_string(), "USERFEED_12345");
///
/// // Create UserFeed with unspecified target
/// let broadcast = Ident::UserFeed(UserId::Unspecified);
/// assert_eq!(broadcast.to_string(), "USERFEED_-1");
/// ```
///
/// Parsing from wire format:
/// ```rust
/// // Parse basic identifiers
/// let cmd: Ident = "REMOTECOMMAND".parse().unwrap();
/// assert_eq!(cmd, Ident::RemoteCommand);
///
/// // Case insensitive parsing
/// let queue: Ident = "remotequeue".parse().unwrap();
/// assert_eq!(queue, Ident::RemoteQueue);
///
/// // Parse UserFeed with target
/// let feed: Ident = "USERFEED_12345".parse().unwrap();
/// assert!(matches!(feed, Ident::UserFeed(_)));
/// ```
///
/// Using in channel configuration:
/// ```rust
/// # use std::num::NonZeroU64;
/// let channel = Channel {
///     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
///     to: UserId::Unspecified,
///     ident: Ident::RemoteCommand,
/// };
///
/// // Channel for targeted user feed
/// let feed_channel = Channel {
///     from: UserId::Id(NonZeroU64::new(12345).unwrap()),
///     to: UserId::Id(NonZeroU64::new(67890).unwrap()),
///     ident: Ident::UserFeed(UserId::Id(NonZeroU64::new(67890).unwrap())),
/// };
/// ```
///
/// # Error Handling
///
/// When parsing from a string, several error conditions are handled:
/// * Unknown identifier types
/// * Missing user ID for `UserFeed`
/// * Invalid user ID format
/// * Malformed wire format
///
/// ```rust
/// // Error cases
/// assert!("UNKNOWN".parse::<Ident>().is_err());
/// assert!("USERFEED".parse::<Ident>().is_err());  // Missing user ID
/// assert!("USERFEED_abc".parse::<Ident>().is_err());  // Invalid user ID
/// assert!("REMOTECOMMAND_12345".parse::<Ident>().is_err());  // Unexpected suffix
/// ```
///
/// # Wire Protocol Constants
///
/// The following constants define the wire protocol values:
/// * `REMOTE_COMMAND` = "REMOTECOMMAND"
/// * `REMOTE_DISCOVER` = "REMOTEDISCOVER"
/// * `REMOTE_QUEUE` = "REMOTEQUEUE"
/// * `STREAM` = "STREAM"
/// * `USER_FEED` = "USERFEED"
///
/// # Serialization
///
/// The type implements [`Display`] and [`FromStr`] traits for string conversion,
/// and supports Serde serialization through [`SerializeDisplay`] and [`DeserializeFromStr`].
///
/// ```rust
/// use serde_json;
///
/// let cmd = Ident::RemoteCommand;
/// let json = serde_json::to_string(&cmd).unwrap();
/// assert_eq!(json, "\"REMOTECOMMAND\"");
///
/// let parsed: Ident = serde_json::from_str(&json).unwrap();
/// assert_eq!(parsed, cmd);
/// ```
///
/// # Notes
///
/// * All wire format strings are converted to uppercase before parsing
/// * `UserFeed` requires a valid user ID suffix
/// * The separator character for `UserFeed` is inherited from [`Channel::SEPARATOR`]
///
/// [`Display`]: std::fmt::Display
/// [`FromStr`]: std::str::FromStr
/// [`SerializeDisplay`]: serde_with::SerializeDisplay
/// [`DeserializeFromStr`]: serde_with::DeserializeFromStr
#[derive(Copy, Clone, Debug, Hash, SerializeDisplay, DeserializeFromStr, PartialEq, Eq)]
pub enum Ident {
    /// Playback control and status messages.
    ///
    /// Used for commands like play, pause, seek, and volume control,
    /// as well as status updates about the current playback state.
    RemoteCommand,

    /// Device discovery and connection management messages.
    ///
    /// Handles device availability announcements, connection requests,
    /// and connection state management.
    RemoteDiscover,

    /// Playback queue publications and UI refresh signals.
    ///
    /// Used to:
    /// * Publish complete queue contents from the controlling device
    /// * Signal receiving devices to refresh their queue display
    RemoteQueue,

    /// Playback reporting messages.
    ///
    /// Transmits playback data for:
    /// * Monetization tracking
    /// * Usage analytics
    /// * Play count tracking
    Stream,

    /// User-targeted social interaction messages.
    ///
    /// Handles social features like following, commenting, and sharing,
    /// with an associated target user ID.
    UserFeed(UserId),
}

impl Channel {
    /// The separator character used in the wire format to delimit channel components.
    ///
    /// This character (`'_'`) separates the `from`, `to`, and `ident` parts when
    /// serializing/deserializing channels.
    const SEPARATOR: char = '_';
}

impl Ident {
    /// Wire format value for playback control and status messages.
    ///
    /// Used to identify messages related to playback control (play, pause, seek)
    /// and device status information.
    const REMOTE_COMMAND: &'static str = "REMOTECOMMAND";

    /// Wire format value for device discovery messages.
    ///
    /// Used to identify messages related to Deezer Connect device discovery
    /// and connection management.
    const REMOTE_DISCOVER: &'static str = "REMOTEDISCOVER";

    /// Wire format value for queue publication messages.
    ///
    /// Used to identify messages containing complete playback queue updates
    /// and UI refresh signals.
    const REMOTE_QUEUE: &'static str = "REMOTEQUEUE";

    /// Wire format value for playback reporting messages.
    ///
    /// Used to identify messages containing playback data for monetization
    /// and analytics tracking.
    const STREAM: &'static str = "STREAM";

    /// Wire format value for social interaction messages.
    ///
    /// Used to identify messages related to social features. Must be followed
    /// by a user ID in the wire format (e.g., "`USERFEED_12345`").
    const USER_FEED: &'static str = "USERFEED";
}

impl fmt::Display for Channel {
    /// Formats an identifier for wire protocol transmission.
    ///
    /// # Examples
    ///
    /// Basic identifiers:
    /// ```rust
    /// assert_eq!(Ident::RemoteCommand.to_string(), "REMOTECOMMAND");
    /// assert_eq!(Ident::Stream.to_string(), "STREAM");
    /// ```
    ///
    /// `UserFeed` with target:
    /// ```rust
    /// use std::num::NonZeroU64;
    ///
    /// let feed = Ident::UserFeed(UserId::Id(NonZeroU64::new(12345).unwrap()));
    /// assert_eq!(feed.to_string(), "USERFEED_12345");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}{}{}",
            self.from,
            Self::SEPARATOR,
            self.to,
            Self::SEPARATOR,
            self.ident
        )
    }
}

impl FromStr for Channel {
    type Err = Error;

    /// Parses a wire format string into an identifier.
    ///
    /// The parsing is case-insensitive for the identifier part. For `UserFeed`,
    /// a valid user ID must follow the separator.
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Basic identifiers (case insensitive)
    /// assert_eq!("REMOTECOMMAND".parse::<Ident>()?, Ident::RemoteCommand);
    /// assert_eq!("stream".parse::<Ident>()?, Ident::Stream);
    ///
    /// // UserFeed with target
    /// let feed: Ident = "USERFEED_12345".parse()?;
    /// assert!(matches!(feed, Ident::UserFeed(_)));
    ///
    /// // Error cases
    /// assert!("UNKNOWN".parse::<Ident>().is_err());
    /// assert!("USERFEED".parse::<Ident>().is_err()); // Missing user ID
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The identifier is not recognized
    /// * `UserFeed` is missing a user ID
    /// * `UserFeed` has an invalid user ID format
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut parts = s.split(Self::SEPARATOR);

        let from = parts.next().ok_or_else(|| {
            Self::Err::invalid_argument("channel string slice should hold `from` part".to_string())
        })?;
        let from = from.parse::<UserId>()?;

        let to = parts.next().ok_or_else(|| {
            Self::Err::invalid_argument("channel string slice should hold `to` part".to_string())
        })?;
        let to = to.parse::<UserId>()?;

        let ident = parts.next().ok_or_else(|| {
            Self::Err::invalid_argument("channel string slice should hold `ident` part".to_string())
        })?;
        let mut ident = ident.to_string();
        if let Some(user_id) = parts.next() {
            write!(ident, "{}{}", Self::SEPARATOR, user_id)?;
        }
        let ident = ident.parse::<Ident>()?;

        if parts.next().is_some() {
            return Err(Self::Err::unimplemented(format!(
                "channel string slice holds unknown trailing parts: `{s}`"
            )));
        }

        Ok(Self { from, to, ident })
    }
}

impl fmt::Display for UserId {
    /// Formats a user ID for wire protocol transmission.
    ///
    /// # Format
    /// * [`UserId::Id`] - Formats as the positive integer value
    /// * [`UserId::Unspecified`] - Formats as "-1"
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::num::NonZeroU64;
    ///
    /// // Specific user ID
    /// let user = UserId::Id(NonZeroU64::new(12345).unwrap());
    /// assert_eq!(user.to_string(), "12345");
    ///
    /// // Unspecified user
    /// let unspec = UserId::Unspecified;
    /// assert_eq!(unspec.to_string(), "-1");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(f, "{id}"),
            Self::Unspecified => write!(f, "-1"),
        }
    }
}

impl FromStr for UserId {
    type Err = Error;

    /// Parses a wire format string into a user ID.
    ///
    /// # Format
    /// * Positive integers - Parsed as [`UserId::Id`]
    /// * "-1" - Parsed as [`UserId::Unspecified`]
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Parse specific user ID
    /// let user: UserId = "12345".parse()?;
    /// assert!(matches!(user, UserId::Id(_)));
    ///
    /// // Parse unspecified user
    /// let unspec: UserId = "-1".parse()?;
    /// assert_eq!(unspec, UserId::Unspecified);
    ///
    /// // Error cases
    /// assert!("0".parse::<UserId>().is_err());  // Zero is invalid
    /// assert!("-2".parse::<UserId>().is_err()); // Only -1 is valid negative
    /// assert!("abc".parse::<UserId>().is_err()); // Must be numeric
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The string is not a valid integer
    /// * The value is zero
    /// * The value is negative but not -1
    /// * The value exceeds `u64::MAX`
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s == "-1" {
            return Ok(Self::Unspecified);
        }

        let id = s.parse::<NonZeroU64>()?;
        Ok(Self::Id(id))
    }
}

impl From<NonZeroU64> for UserId {
    /// Creates a [`UserId::Id`] from a [`NonZeroU64`].
    ///
    /// This conversion is infallible as [`NonZeroU64`] guarantees
    /// a valid positive, non-zero value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::num::NonZeroU64;
    ///
    /// let id = NonZeroU64::new(12345).unwrap();
    /// let user: UserId = id.into();
    /// assert!(matches!(user, UserId::Id(_)));
    /// ```
    fn from(id: NonZeroU64) -> Self {
        Self::Id(id)
    }
}

impl fmt::Display for Ident {
    /// Formats a message identifier for wire protocol transmission.
    ///
    /// Basic identifiers are formatted as simple uppercase strings.
    /// The [`Ident::UserFeed`] variant includes the target user ID
    /// after a separator.
    ///
    /// # Examples
    ///
    /// Basic identifiers:
    /// ```rust
    /// let cmd = Ident::RemoteCommand;
    /// assert_eq!(cmd.to_string(), "REMOTECOMMAND");
    ///
    /// let stream = Ident::Stream;
    /// assert_eq!(stream.to_string(), "STREAM");
    /// ```
    ///
    /// `UserFeed` with target:
    /// ```rust
    /// use std::num::NonZeroU64;
    ///
    /// let feed = Ident::UserFeed(UserId::Id(NonZeroU64::new(12345).unwrap()));
    /// assert_eq!(feed.to_string(), "USERFEED_12345");
    ///
    /// let broadcast = Ident::UserFeed(UserId::Unspecified);
    /// assert_eq!(broadcast.to_string(), "USERFEED_-1");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteCommand => write!(f, "{}", Self::REMOTE_COMMAND),
            Self::RemoteDiscover => write!(f, "{}", Self::REMOTE_DISCOVER),
            Self::RemoteQueue => write!(f, "{}", Self::REMOTE_QUEUE),
            Self::Stream => write!(f, "{}", Self::STREAM),
            Self::UserFeed(id) => write!(f, "{}{}{}", Self::USER_FEED, Channel::SEPARATOR, id),
        }
    }
}

impl FromStr for Ident {
    type Err = Error;

    /// Parses a wire format string into a message identifier.
    ///
    /// The identifier part is parsed case-insensitively. For [`Ident::UserFeed`],
    /// a valid user ID must follow the separator.
    ///
    /// # Wire Format
    ///
    /// Basic identifiers:
    /// * "REMOTECOMMAND" - Playback control
    /// * "REMOTEDISCOVER" - Device discovery
    /// * "REMOTEQUEUE" - Queue publications
    /// * "STREAM" - Playback reporting
    ///
    /// `UserFeed` format:
    /// * "USERFEED_<`user_id`>" where `user_id` is either:
    ///   * A positive integer (specific user)
    ///   * "-1" (unspecified user)
    ///
    /// # Examples
    ///
    /// Basic identifiers (case insensitive):
    /// ```rust
    /// assert_eq!("REMOTECOMMAND".parse::<Ident>()?, Ident::RemoteCommand);
    /// assert_eq!("remotecommand".parse::<Ident>()?, Ident::RemoteCommand);
    /// assert_eq!("stream".parse::<Ident>()?, Ident::Stream);
    /// ```
    ///
    /// `UserFeed` variants:
    /// ```rust
    /// // Specific target
    /// let feed: Ident = "USERFEED_12345".parse()?;
    /// assert!(matches!(feed, Ident::UserFeed(_)));
    ///
    /// // Broadcast
    /// let broadcast: Ident = "USERFEED_-1".parse()?;
    /// assert!(matches!(broadcast, Ident::UserFeed(UserId::Unspecified)));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The identifier is not one of the known types
    /// * `UserFeed` is missing its required user ID
    /// * `UserFeed` has an invalid user ID format
    /// * The format includes unexpected additional parts
    ///
    /// ```rust
    /// // Unknown identifier
    /// assert!("UNKNOWN".parse::<Ident>().is_err());
    ///
    /// // Missing UserFeed target
    /// assert!("USERFEED".parse::<Ident>().is_err());
    ///
    /// // Invalid UserFeed target
    /// assert!("USERFEED_abc".parse::<Ident>().is_err());
    /// assert!("USERFEED_0".parse::<Ident>().is_err());
    /// assert!("USERFEED_-2".parse::<Ident>().is_err());
    /// ```
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (ident, user_id) = s
            .split_once('_')
            .map_or((s, None), |split| (split.0, Some(split.1)));

        let ident = ident.to_uppercase();
        let variant = match ident.as_ref() {
            Self::REMOTE_COMMAND => Self::RemoteCommand,
            Self::REMOTE_DISCOVER => Self::RemoteDiscover,
            Self::REMOTE_QUEUE => Self::RemoteQueue,
            Self::STREAM => Self::Stream,
            Self::USER_FEED => {
                if let Some(user_id) = user_id {
                    let user_id = user_id.parse::<UserId>()?;
                    Self::UserFeed(user_id)
                } else {
                    return Err(Self::Err::invalid_argument(format!(
                        "message identifier `{ident}` should have user id suffix"
                    )));
                }
            }
            _ => {
                return Err(Self::Err::unimplemented(format!(
                    "message identifier `{s}`"
                )))
            }
        };

        Ok(variant)
    }
}
