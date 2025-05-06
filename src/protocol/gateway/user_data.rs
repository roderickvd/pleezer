//! User data and settings from Deezer's gateway API.
//!
//! This module handles user-specific information including:
//! * Authentication tokens and licenses
//! * User preferences and settings
//! * Media server configuration
//! * Feature flags (gatekeeps)
//!
//! # Wire Format
//!
//! Response format:
//! ```json
//! {
//!     "USER": {
//!         "USER_ID": "123456789",
//!         "BLOG_NAME": "Username",
//!         "OPTIONS": {
//!             "license_token": "secret",
//!             "too_many_devices": false,
//!             "expiration_timestamp": 1234567890,
//!             "ads_audio": false
//!         },
//!         "AUDIO_SETTINGS": {
//!             "connected_device_streaming_preset": "lossless"
//!         }
//!     },
//!     "USER_TOKEN": "secret_token",
//!     "checkForm": "api_token",
//!     "__DZR_GATEKEEPS__": {
//!         "remote_control": true
//!     },
//!     "URL_MEDIA": "https://media.deezer.com",
//!     "GAIN": {
//!         "TARGET": "-15"
//!     }
//! }
//! ```

use std::{ops::Deref, str::FromStr, time::SystemTime};

use serde::Deserialize;
use serde_with::{DisplayFromStr, PickFirst, TimestampSeconds, formats::Flexible, serde_as};
use url::Url;
use veil::Redact;

use crate::protocol::{self, connect::UserId};

use super::Method;

/// Gateway method name for retrieving user data.
///
/// Returns complete user information including:
/// * Profile and preferences
/// * Authentication tokens
/// * Feature flags
/// * Media server configuration
impl Method for UserData {
    const METHOD: &'static str = "deezer.getUserData";
}

/// Complete user data from Deezer's gateway.
///
/// Contains all user-specific information needed for authentication
/// and playback configuration.
// TODO : #[serde(rename_all = "UPPERCASE")]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Redact, Hash)]
pub struct UserData {
    /// User profile and preferences
    #[serde(rename = "USER")]
    pub user: User,

    /// Authentication token for user operations
    #[serde(rename = "USER_TOKEN")]
    #[redact]
    pub user_token: String,

    /// API authentication token
    #[serde(rename = "checkForm")]
    #[redact]
    pub api_token: String,

    /// Feature flags and capabilities
    #[serde(default)]
    #[serde(rename = "__DZR_GATEKEEPS__")]
    pub gatekeeps: Gatekeeps,

    /// Media server URL
    #[serde(default)]
    #[serde(rename = "URL_MEDIA")]
    pub media_url: MediaUrl,

    /// Volume normalization settings
    #[serde(default)]
    #[serde(rename = "GAIN")]
    pub gain: Gain,
}

/// Media server URL wrapper.
///
/// Provides type-safe handling of the Deezer media server URL.
/// Defaults to "<https://media.deezer.com>".
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct MediaUrl(pub Url);

/// Provides read-only access to the underlying URL.
///
/// # Examples
///
/// ```rust
/// use deezer::gateway::MediaUrl;
///
/// let url = MediaUrl::default();
/// assert_eq!(url.scheme(), "https");
/// assert_eq!(url.host_str(), Some("media.deezer.com"));
/// ```
impl Deref for MediaUrl {
    type Target = Url;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Converts a `MediaUrl` into a standard `Url`.
///
/// This conversion consumes the `MediaUrl` wrapper and returns
/// the underlying URL.
///
/// # Examples
///
/// ```rust
/// use url::Url;
/// use deezer::gateway::MediaUrl;
///
/// let media_url = MediaUrl::default();
/// let url: Url = media_url.into();
/// ```
impl From<MediaUrl> for Url {
    #[inline]
    fn from(url: MediaUrl) -> Self {
        url.0
    }
}

/// Creates the default Deezer media server URL.
///
/// Returns a `MediaUrl` wrapping "<https://media.deezer.com>".
/// This URL is used when none is specified in the API response.
///
/// # Panics
///
/// Never panics as the URL is statically validated.
///
/// # Examples
///
/// ```rust
/// use deezer::gateway::MediaUrl;
///
/// let url = MediaUrl::default();
/// assert_eq!(url.as_str(), "https://media.deezer.com");
/// ```
impl Default for MediaUrl {
    #[inline]
    fn default() -> Self {
        let media_url = Url::from_str("https://media.deezer.com").expect("invalid media url");
        Self(media_url)
    }
}

/// User profile and preferences.
///
/// Contains user identification and settings.
#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct User {
    /// Unique user identifier
    #[serde(rename = "USER_ID")]
    #[serde_as(as = "PickFirst<(_, DisplayFromStr)>")]
    pub id: UserId,

    /// Display name (defaults to "UNKNOWN")
    #[serde(default)]
    #[serde(rename = "BLOG_NAME")]
    pub name: String,

    /// License and device management
    #[serde(rename = "OPTIONS")]
    pub options: Options,

    /// Audio quality preferences
    #[serde(default)]
    #[serde(rename = "AUDIO_SETTINGS")]
    pub audio_settings: AudioSettings,
}

/// User license and device management options.
///
/// Contains settings related to:
/// * License tokens and expiration
/// * Device limitations
/// * Audio ads configuration
#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Redact, Hash)]
pub struct Options {
    /// License authentication token
    #[redact]
    pub license_token: String,

    /// Whether user has exceeded device limit
    #[serde(default)]
    pub too_many_devices: bool,

    /// License expiration time
    #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
    pub expiration_timestamp: SystemTime,

    /// Whether to play ads in audio streams
    #[serde(default)]
    pub ads_audio: bool,
}

/// Audio quality settings.
#[serde_as]
#[derive(Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct AudioSettings {
    /// Preferred audio quality for connected devices
    #[serde_as(as = "DisplayFromStr")]
    pub connected_device_streaming_preset: protocol::connect::AudioQuality,
}

/// Feature flags and capabilities.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Gatekeeps {
    /// Whether remote control is enabled
    #[serde(default)]
    pub remote_control: Option<bool>,
}

/// Creates default feature flags and capabilities.
///
/// Sets remote control to `None`.
///
/// # Examples
///
/// ```rust
/// use deezer::gateway::Gatekeeps;
///
/// let gatekeeps = Gatekeeps::default();
/// assert_eq!(gatekeeps.remote_control, None);
/// ```
impl Default for Gatekeeps {
    #[inline]
    fn default() -> Self {
        Self {
            remote_control: None,
        }
    }
}

/// Volume normalization settings.
#[serde_as]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Gain {
    /// Target loudness level in decibels
    ///
    /// Default value is -15 dB.
    #[serde(default)]
    #[serde(rename = "TARGET")]
    #[serde_as(as = "PickFirst<(DisplayFromStr, _)>")]
    pub target: i64,
}

/// Creates default volume normalization settings.
///
/// Sets a target loudness of -15 dB, which is Deezer's
/// standard normalization target.
///
/// # Examples
///
/// ```rust
/// use deezer::gateway::Gain;
///
/// let gain = Gain::default();
/// assert_eq!(gain.target, -15);
/// ```
impl Default for Gain {
    #[inline]
    fn default() -> Self {
        Self { target: -15 }
    }
}
