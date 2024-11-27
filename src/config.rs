use regex_lite::Regex;
use uuid::Uuid;

use crate::{
    arl::Arl,
    decrypt::{Key, KEY_LENGTH},
    error::{Error, Result},
    http,
    protocol::connect::DeviceType,
};

/// Methods that can be used to authenticate with Deezer.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Credentials {
    /// The user's email and password.
    Login {
        /// The user's email.
        email: String,
        /// The user's password.
        password: String,
    },

    /// The user's `arl` token.
    Arl(Arl),
}

/// The configuration of pleezer.
// TODO: implement Debug manually to avoid leaking the arl.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd)]
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
}

impl Config {
    /// The checksum of the secret key used to decrypt tracks. This is *not* the actual key,
    /// but used to verify that some supplied key is correct.
    pub const BF_SECRET_MD5: &'static str = "7ebf40da848f4a0fb3cc56ddbe6c2d09";

    /// The URL of the Deezer web player, used to retrieve the `app-web` source from which
    /// the secret key is extracted.
    const WEB_PLAYER_URL: &'static str = "https://www.deezer.com/en/channels/explore/";

    /// Get the decryption key from the Deezer web player.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The web player source could not be retrieved.
    /// - The app-web source could not be found.
    /// - The secret key could not be found or parsed.
    #[expect(clippy::missing_panics_doc)]
    pub async fn try_key(client: &http::Client) -> Result<Key> {
        // Get the web player source.
        let source = Self::get_text(client, Self::WEB_PLAYER_URL).await?;

        // Find the URL of the app-web source.
        let re = Regex::new(r"https:\/\/.+\/app-web.*\.js").unwrap();
        let url = re
            .find(&source)
            .ok_or(Error::not_found("unable to find app-web source"))?;

        // Get the app-web source.
        let url = url.as_str();
        trace!("bootstrapping from {url}");
        let source = Self::get_text(client, url).await?;

        // Find the Blowfish decryption key.
        let re = Regex::new(r"0x61%2C(0x[0-9a-f]{2}%2C){6}0x67").unwrap();
        let a = re
            .find(&source)
            .ok_or(Error::not_found("unable to find first half of secret key"))?;
        let re = Regex::new(r"0x31%2C(0x[0-9a-f]{2}%2C){6}0x34").unwrap();
        let b = re
            .find(&source)
            .ok_or(Error::not_found("unable to find second half of secret key"))?;

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

    /// Get the body text of a URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL could not be parsed or the request failed.
    async fn get_text(client: &http::Client, url: &str) -> Result<String> {
        let url = url.parse::<reqwest::Url>()?;
        let request = client.get(url, "");
        let response = client.execute(request).await?;
        response.text().await.map_err(Into::into)
    }

    /// Convert a half key from the `app-web` source to a format and ordering suitable for
    /// constructing the full key.
    ///
    /// # Errors
    ///
    /// Returns an error if the half key does not contain the right amount of valid characters.
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
