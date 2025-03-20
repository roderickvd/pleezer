//! Media streaming types and formats for Deezer.
//!
//! This module handles media access requests and responses, including:
//! * Track/episode download URLs
//! * Audio formats and quality levels
//! * Content encryption
//! * Access tokens and expiry
//! * External streaming URLs (podcasts)
//! * Multiple quality streams (radio)
//!
//! # Authentication
//!
//! Media access requires two types of tokens:
//! * License token - For general media access rights
//! * Track tokens - For specific content access
//!
//! Both tokens have expiration times and must be refreshed periodically.
//!
//! # Content Types
//!
//! Three main content categories:
//! * Regular tracks - Encrypted, quality selection, token auth
//! * External content - No encryption, direct URLs
//! * Previews - Short samples, usually no encryption
//!
//! Each type may have different authentication and delivery requirements.
//!
//! # Error Handling
//!
//! Media access can fail in several ways:
//! * Authentication errors (invalid/expired tokens)
//! * Availability errors (geo-restrictions, takedowns)
//! * Technical errors (network issues, invalid formats)
//!
//! Errors include both a code and human-readable message.
//! Common error codes:
//! * 404 - Content not found
//! * 403 - Access denied
//! * 429 - Too many requests
//!
//! # Wire Format
//!
//! Request:
//! ```json
//! {
//!     "license_token": "secret",
//!     "media": [{
//!         "type": "FULL",
//!         "formats": [{
//!             "cipher": "BF_CBC_STRIPE",
//!             "format": "MP3_320"
//!         }]
//!     }],
//!     "track_tokens": ["token1", "token2"]
//! }
//! ```
//!
//! Response:
//! ```json
//! {
//!     "data": [{
//!         "media": [{
//!             "media_type": "FULL",
//!             "cipher": {"type": "BF_CBC_STRIPE"},
//!             "format": "MP3_320",
//!             "sources": [{
//!                 "url": "https://...",
//!                 "provider": "cdn"
//!             }],
//!             "nbf": 1234567890,
//!             "exp": 1234599999
//!         }]
//!     }]
//! }
//! ```

use std::{fmt, time::SystemTime};

use serde::{Deserialize, Serialize};
use serde_with::{TimestampSeconds, formats::Flexible, serde_as};
use url::Url;
use veil::Redact;

use super::connect::AudioQuality;

/// Media access request.
///
/// Used to request streaming URLs for tracks with specific
/// format and encryption requirements.
#[derive(Clone, Eq, PartialEq, Serialize, Debug, Hash)]
pub struct Request {
    /// Authentication token for accessing licensed content
    pub license_token: String,
    /// List of requested media formats and types
    pub media: Vec<Media>,
    /// Authentication tokens for specific tracks
    /// One token per requested track
    pub track_tokens: Vec<String>,
}

/// Media format request.
///
/// Specifies the desired media type (full/preview) and formats
/// with their encryption methods. Multiple format/cipher combinations
/// can be requested to handle fallback scenarios.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Debug, Hash)]
pub struct Media {
    /// Content type requested (full track or preview)
    /// Defaults to full track
    #[serde(default)]
    #[serde(rename = "type")]
    pub typ: Type,

    /// List of format and encryption combinations to try
    /// Ordered by preference (first is most preferred)
    #[serde(rename = "formats")]
    pub cipher_formats: Vec<CipherFormat>,
}

/// Media content type.
///
/// Determines whether to return the full track or just
/// a preview clip.
#[derive(Copy, Clone, Default, Eq, PartialEq, Deserialize, Serialize, Debug, Hash)]
pub enum Type {
    /// Full-length track
    #[default]
    FULL,
    /// Preview clip (typically 30 seconds)
    PREVIEW,
}

/// Formats the media type for display.
///
/// Shows either "FULL" or "PREVIEW" matching the protocol's
/// string representation.
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::Type;
///
/// assert_eq!(Type::FULL.to_string(), "FULL");
/// assert_eq!(Type::PREVIEW.to_string(), "PREVIEW");
/// ```
impl fmt::Display for Type {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Format and encryption combination.
///
/// Specifies both the audio format (quality level) and
/// encryption method for the content. Used to request
/// specific quality/security combinations.
///
/// For livestreams, only `EXTERNAL` format with `NONE` cipher is used
/// since radio streams use direct unencrypted URLs.
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::{CipherFormat, Cipher, Format};
///
/// // For regular tracks
/// let format = CipherFormat {
///     cipher: Cipher::BF_CBC_STRIPE,
///     format: Format::MP3_320,
/// };
///
/// // For livestreams
/// let format = CipherFormat {
///     cipher: Cipher::NONE,
///     format: Format::EXTERNAL,
/// };
/// ```
#[derive(
    Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash,
)]
pub struct CipherFormat {
    /// Encryption method to use for content protection
    pub cipher: Cipher,
    /// Audio format and quality level requested
    pub format: Format,
}

/// Content encryption method.
#[derive(
    Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash,
)]
#[expect(non_camel_case_types)]
pub enum Cipher {
    /// Blowfish CBC encryption with data striping
    /// Used for most protected content
    #[default]
    BF_CBC_STRIPE,

    /// No encryption
    /// Used for external content and previews
    NONE,
}

/// Formats the cipher type for display.
///
/// Shows either "`BF_CBC_STRIPE`" or "NONE" matching the protocol's
/// string representation.
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::Cipher;
///
/// assert_eq!(Cipher::BF_CBC_STRIPE.to_string(), "BF_CBC_STRIPE");
/// assert_eq!(Cipher::NONE.to_string(), "NONE");
/// ```
impl fmt::Display for Cipher {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Audio format and quality level.
///
/// Represents different audio formats and their quality levels,
/// mapped to specific numeric IDs in the protocol.
#[derive(
    Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash,
)]
#[expect(non_camel_case_types)]
#[repr(i64)]
pub enum Format {
    /// External source hosted outside Deezer's CDN
    /// Protocol ID: -1
    EXTERNAL = -1,

    /// Free Lossless Audio Codec
    /// Highest quality, largest file size
    /// Protocol ID: 9
    FLAC = 9,

    /// MP3 at 64 kbps
    /// Basic quality, smallest file size
    /// Protocol ID: 10
    MP3_64 = 10,

    /// MP3 at 128 kbps
    /// Standard quality, balanced size
    /// Protocol ID: 1
    #[default]
    MP3_128 = 1,

    /// MP3 at 320 kbps
    /// High quality, larger file size
    /// Protocol ID: 3
    MP3_320 = 3,

    /// MP3 with unknown or variable bitrate
    /// Protocol ID: 0
    MP3_MISC = 0,
}

/// Formats the audio format for display.
///
/// Shows the format name (e.g., "`MP3_320`", "FLAC") matching
/// the protocol's string representation.
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::Format;
///
/// assert_eq!(Format::MP3_320.to_string(), "MP3_320");
/// assert_eq!(Format::FLAC.to_string(), "FLAC");
/// ```
impl fmt::Display for Format {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Converts a media format to its corresponding audio quality level.
///
/// Maps format types to quality levels:
/// * `MP3_64` -> Basic (64 kbps)
/// * `MP3_128` -> Standard (128 kbps)
/// * `MP3_320` -> High (320 kbps)
/// * FLAC -> Lossless
/// * Others -> Unknown
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::{media::Format, connect::AudioQuality};
///
/// assert_eq!(AudioQuality::from(Format::MP3_320), AudioQuality::High);
/// assert_eq!(AudioQuality::from(Format::FLAC), AudioQuality::Lossless);
/// assert_eq!(AudioQuality::from(Format::EXTERNAL), AudioQuality::Unknown);
/// ```
impl From<Format> for AudioQuality {
    #[inline]
    fn from(format: Format) -> Self {
        match format {
            Format::MP3_64 => AudioQuality::Basic,
            Format::MP3_128 => AudioQuality::Standard,
            Format::MP3_320 => AudioQuality::High,
            Format::FLAC => AudioQuality::Lossless,
            _ => AudioQuality::Unknown,
        }
    }
}

/// Media access response.
///
/// Contains either:
/// * Successful media access information with URLs and formats
/// * Error details when access fails
///
/// Multiple media entries may be returned when requesting
/// multiple tracks or formats.
#[derive(Clone, Default, Eq, PartialEq, Deserialize, Serialize, Debug, Hash)]
pub struct Response {
    /// List of media access results or errors
    /// One entry per requested track
    pub data: Vec<Data>,
}

/// Response data variant.
///
/// Can contain either media information or error details.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize, Debug, Hash)]
#[serde(untagged)]
pub enum Data {
    /// Media information, including URLs, formats and validity periods
    Media {
        /// List of available media formats and sources
        media: Vec<Medium>,
    },
    /// Error information when media access fails
    Errors {
        /// List of error details and codes
        errors: Vec<Error>,
    },
}

/// Media access error.
///
/// Represents an error response from the media server, containing
/// both an error code and descriptive message.
///
/// # Wire Format
///
/// ```json
/// {
///     "errors": [{
///         "code": 404,
///         "message": "Track not found"
///     }]
/// }
/// ```
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::Error;
///
/// let error = Error {
///     code: 404,
///     message: "Track not found".to_string(),
/// };
///
/// // Displays as: "Track not found (404)"
/// println!("{}", error);
/// ```
#[derive(Clone, Eq, Default, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub struct Error {
    /// Numeric error code
    code: i64,
    /// Human-readable error description
    message: String,
}

/// Formats an error for display.
///
/// Shows both the error message and code in the format:
/// `"{message} ({code})"`
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::Error;
///
/// let error = Error {
///     code: 404,
///     message: "Not found".to_string(),
/// };
/// assert_eq!(error.to_string(), "Not found (404)");
/// ```
impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// Media access details.
///
/// Contains all information needed to access a media file,
/// including URLs, format, and validity period.
#[serde_as]
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize, Debug, Hash)]
pub struct Medium {
    /// Type of media content (full track or preview)
    #[serde(default)]
    pub media_type: Type,

    /// Content encryption configuration
    /// Specifies the cipher type used to protect the content
    #[serde(default)]
    pub cipher: CipherType,

    /// Audio format and quality level
    /// Indicates bitrate and codec for the content
    #[serde(default)]
    pub format: Format,

    /// List of available download sources
    /// Multiple sources may be provided for redundancy
    pub sources: Vec<Source>,

    /// Time before which content is not accessible
    /// Used for release date restrictions
    #[serde(rename = "nbf")]
    #[serde_as(as = "Option<TimestampSeconds<i64, Flexible>>")]
    pub not_before: Option<SystemTime>,

    /// Time after which content becomes inaccessible
    /// Used for token expiration and temporary access
    #[serde(rename = "exp")]
    #[serde_as(as = "Option<TimestampSeconds<i64, Flexible>>")]
    pub expiry: Option<SystemTime>,
}

/// Encryption method wrapper for media content.
///
/// Used in responses to specify how the media content is encrypted.
/// The wrapper structure matches the protocol's JSON format.
///
/// # Wire Format
///
/// ```json
/// {
///     "type": "BF_CBC_STRIPE"  // or "NONE"
/// }
/// ```
///
/// # Examples
///
/// ```rust
/// use deezer::protocol::media::{Cipher, CipherType};
///
/// let cipher = CipherType {
///     typ: Cipher::BF_CBC_STRIPE,
/// };
///
/// // Default is BF_CBC_STRIPE
/// assert_eq!(CipherType::default().typ, Cipher::BF_CBC_STRIPE);
/// ```
#[derive(
    Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash,
)]
pub struct CipherType {
    /// The encryption method to use
    #[serde(rename = "type")]
    pub typ: Cipher,
}

/// Media source information.
///
/// Contains URL and provider information for content delivery:
/// * URLs are redacted in debug output for security
/// * Provider indicates delivery network (e.g., "cdn")
///
/// Multiple sources may be available for redundancy.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Redact, Hash)]
pub struct Source {
    /// Download URL for the media content
    /// Redacted in debug output for security
    #[redact]
    pub url: Url,

    /// Content delivery provider identifier
    /// Usually "cdn" for Deezer's content delivery network
    #[serde(default)]
    pub provider: String,
}
