//! Configuration and authentication for pleezer.
//!
//! This module handles:
//! * Authentication methods (email/password or ARL)
//! * Device identification and settings
//! * Network configuration (interface binding)
//! * Audio configuration (volume, normalization)
//! * Track decryption configuration
//! * API client settings
//!
//! # Examples
//!
//! ```rust
//! use pleezer::config::{Config, Credentials};
//! use pleezer::arl::Arl;
//! use pleezer::protocol::connect::Percentage;
//! use std::net::IpAddr;
//!
//! // Configure with ARL authentication, initial volume, and specific network binding
//! let config = Config {
//!     credentials: Credentials::Arl(arl),
//!     device_name: "My Player".to_string(),
//!     normalization: true,
//!     initial_volume: Some(Percentage::from_percent_f32(50.0)), // Start at 50% volume
//!     bind_address: "192.168.1.2".parse().unwrap(), // Bind to specific interface
//!     // ... other settings ...
//! };
//!
//! // Configure with email/password
//! let config = Config {
//!     credentials: Credentials::Login {
//!         email: "user@example.com".to_string(),
//!         password: "secret".to_string(),
//!     },
//!     // ... other settings ...
//! };
//! ```

use std::net::IpAddr;

use regex_lite::Regex;
use uuid::Uuid;
use veil::Redact;

use crate::{
    arl::Arl,
    decrypt::{KEY_LENGTH, Key},
    error::{Error, Result},
    http,
    protocol::connect::{DeviceType, Percentage},
};

/// Authentication methods for Deezer.
///
/// Supports either email/password login or ARL token authentication.
/// Email/password is preferred as these credentials can be used to
/// obtain fresh tokens, while ARLs expire and cannot be refreshed.
///
/// # Security
///
/// Passwords and ARL tokens are automatically redacted in debug output
/// to prevent accidental credential exposure.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Redact)]
pub enum Credentials {
    /// Email and password authentication.
    ///
    /// Recommended method as it allows automatic token refresh.
    Login {
        /// User's Deezer account email
        email: String,
        /// User's Deezer account password
        #[redact]
        password: String,
    },

    /// Authentication Reference Link token.
    ///
    /// A pre-authenticated token that grants temporary access.
    /// Will need manual replacement when it expires.
    #[redact(all)]
    Arl(Arl),
}

/// Complete configuration for pleezer.
///
/// Contains all settings needed to:
/// * Authenticate with Deezer
/// * Identify the device
/// * Configure playback behavior
/// * Set up API access
///
/// Most settings have reasonable defaults that can be overridden
/// as needed.
#[derive(Clone, PartialEq, PartialOrd, Debug)]
pub struct Config {
    /// The name of the application.
    ///
    /// By default this is retrieved from `Cargo.toml`, used in the
    /// `User-Agent` string, and the fallback device name if not provided and
    /// the system hostname is not available.
    pub app_name: String,

    /// The version of the application.
    ///
    /// By default this is retrieved from `Cargo.toml` used in the `User-Agent`
    /// string.
    pub app_version: String,

    /// The language of the application in ISO 639-1 format.
    ///
    /// By default this is "en" for English, used in the `User-Agent` string,
    /// as well as `Accept-Language`header in API requests.
    pub app_lang: String,

    /// The player's name as it appears to Deezer clients.
    ///
    /// By default this is equal to `app_name`.
    pub device_name: String,

    /// The player's type as it appears to Deezer clients.
    ///
    ///By default this is equal to `DeviceType::Web`.
    pub device_type: DeviceType,

    /// The ID that uniquely identifies the device.
    ///
    /// By default this is the machine ID, or a random UUID if the machine ID
    /// could not be retrieved.
    pub device_id: Uuid,

    /// Whether to normalize the audio.
    ///
    /// By default this is `false`.
    pub normalization: bool,

    /// Initial volume level.
    ///
    /// Used when no volume is reported by Deezer client or when reported as maximum.
    /// None means no volume override.
    pub initial_volume: Option<Percentage>,

    /// Dither bit depth based on DAC linearity (ENOB)
    pub dither_bits: Option<f32>,

    /// Maximum amount of RAM in bytes that can be used for storing audio files.
    /// `None` means use temporary files instead of RAM.
    pub max_ram: Option<u64>,

    /// Whether other clients may take over an existing connection.
    ///
    /// By default this is `true`.
    pub interruptions: bool,

    /// Script to execute when events occur
    pub hook: Option<String>,

    /// The client ID used in API requests.
    ///
    /// By default this is a random number of 9 digits.
    pub client_id: usize,

    /// The `User-Agent` string used in API requests.
    ///
    /// By default this is a combination of the application name, version, and
    /// language, to be like the official Deezer Desktop client.
    pub user_agent: String,

    /// The credentials used to authenticate with Deezer.
    pub credentials: Credentials,

    /// Secret for computing the track decryption key.
    pub bf_secret: Option<Key>,

    /// Whether to eavesdrop on the network traffic.
    pub eavesdrop: bool,

    /// The address to bind for outgoing connections.
    pub bind_address: IpAddr,
}

impl Config {
    /// MD5 checksum of the correct Blowfish secret key.
    ///
    /// Used to verify that an extracted or provided key is valid.
    pub const BF_SECRET_MD5: &'static str = "7ebf40da848f4a0fb3cc56ddbe6c2d09";

    /// URL of Deezer's web player interface.
    ///
    /// Used to locate and extract the app-web JavaScript that
    /// contains the secret key.
    const WEB_PLAYER_URL: &'static str = "https://www.deezer.com/en/channels/explore/";

    /// Attempts to extract the track decryption key from Deezer's web player.
    ///
    /// This method:
    /// 1. Downloads the web player HTML
    /// 2. Locates the app-web JavaScript URL
    /// 3. Downloads the JavaScript
    /// 4. Extracts and assembles the key
    /// 5. Verifies the key against `BF_SECRET_MD5`
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Web player source cannot be retrieved
    /// * App-web JavaScript cannot be found
    /// * Key fragments cannot be located
    /// * Key assembly fails
    /// * Assembled key is invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use pleezer::config::Config;
    /// use pleezer::http;
    ///
    /// let client = http::Client::new();
    /// let key = Config::try_key(&client).await?;
    /// ```
    #[expect(clippy::missing_panics_doc)]
    pub async fn try_key(client: &http::Client) -> Result<Key> {
        // Get the web player source.
        let source = Self::get_text(client, Self::WEB_PLAYER_URL).await?;

        // Find the URL of the app-web source.
        let re = Regex::new(r"https:\/\/.+\/app-web.*\.js").unwrap();
        let url = re
            .find(&source)
            .ok_or_else(|| Error::not_found("unable to find app-web source"))?;

        // Get the app-web source.
        let url = url.as_str();
        trace!("bootstrapping from {url}");
        let source = Self::get_text(client, url).await?;

        // Find the Blowfish decryption key.
        let re = Regex::new(r"0x61%2C(0x[0-9a-f]{2}%2C){6}0x67").unwrap();
        let a = re
            .find(&source)
            .ok_or_else(|| Error::not_found("unable to find first half of secret key"))?;
        let re = Regex::new(r"0x31%2C(0x[0-9a-f]{2}%2C){6}0x34").unwrap();
        let b = re
            .find(&source)
            .ok_or_else(|| Error::not_found("unable to find second half of secret key"))?;

        let a = Self::convert_half(a.as_str())?;
        let b = Self::convert_half(b.as_str())?;

        let mut key = Vec::with_capacity(KEY_LENGTH);
        for i in 0..(KEY_LENGTH / 2) {
            key.push(a[i]);
            key.push(b[i]);
        }

        let key = String::from_utf8_lossy(&key).into_owned();
        key.parse()
    }

    /// Downloads text content from a URL.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * URL is invalid
    /// * Network request fails
    /// * Response isn't valid UTF-8 text
    async fn get_text(client: &http::Client, url: &str) -> Result<String> {
        let url = url.parse::<reqwest::Url>()?;
        let request = client.get(url, "");
        let response = client.execute(request).await?;
        response.text().await.map_err(Into::into)
    }

    /// Converts a key fragment from hex format to bytes.
    ///
    /// Takes a fragment like "0x61%2C0x62%2C..." and converts it
    /// to a sequence of bytes in the correct order for key assembly.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Fragment contains invalid hex values
    /// * Wrong number of bytes extracted (must be 8)
    fn convert_half(half: &str) -> Result<Vec<u8>> {
        let bytes: Vec<u8> = half
            .split("%2C")
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter_map(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .collect();

        let len = bytes.len();
        if len != 8 {
            return Err(Error::out_of_range(format!(
                "half key has {len} valid characters"
            )));
        }

        Ok(bytes)
    }
}
