use std::{num::NonZeroU64, time::SystemTime};

use serde::Deserialize;
use serde_with::{formats::Flexible, serde_as, DisplayFromStr, TimestampSeconds};
use veil::Redact;

use crate::protocol;

use super::Method;

impl Method for UserData {
    const METHOD: &'static str = "deezer.getUserData";
}

// TODO : #[serde(rename_all = "UPPERCASE")]
#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Redact, Hash)]
pub struct UserData {
    #[serde(rename = "USER")]
    pub user: User,
    #[serde(rename = "USER_TOKEN")]
    #[redact]
    pub user_token: String,
    #[serde(rename = "checkForm")]
    #[redact]
    pub api_token: String,
    #[serde(default)]
    #[serde(rename = "__DZR_GATEKEEPS__")]
    pub gatekeeps: Gatekeeps,
    #[serde(rename = "URL_MEDIA")]
    pub media_url: Option<String>,
    #[serde(default)]
    #[serde(rename = "GAIN")]
    pub gain: Gain,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct User {
    #[serde(rename = "USER_ID")]
    // TODO : replace with UserId
    pub id: NonZeroU64,
    #[serde(rename = "BLOG_NAME")]
    pub name: Option<String>,
    #[serde(rename = "OPTIONS")]
    pub options: Options,
    #[serde(default)]
    #[serde(rename = "AUDIO_SETTINGS")]
    pub audio_settings: AudioSettings,
    #[serde(default)]
    #[serde(rename = "SETTING")]
    pub settings: Settings,
}

// TODO: find out how to register our own device.

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Redact, Hash)]
pub struct Options {
    #[redact]
    pub license_token: String,
    #[serde(default)]
    pub too_many_devices: bool,
    #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
    pub expiration_timestamp: SystemTime,
    #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
    pub timestamp: SystemTime,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct AudioSettings {
    pub connected_device_streaming_preset: String,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            connected_device_streaming_preset: protocol::connect::AudioQuality::Standard
                .to_string(),
        }
    }
}

#[derive(Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Settings {
    pub site: SiteSettings,
}

#[derive(Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct SiteSettings {
    pub player_normalize: bool,
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Gatekeeps {
    // disable_device_limitation: bool,
    pub volume_normalization: bool,
    pub remote_control: bool,
}

impl Default for Gatekeeps {
    fn default() -> Self {
        Self {
            volume_normalization: true,
            remote_control: true,
        }
    }
}

#[serde_as]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Gain {
    #[serde(default)]
    #[serde(rename = "TARGET")]
    #[serde_as(as = "DisplayFromStr")]
    pub target: i64,
}

impl Default for Gain {
    fn default() -> Self {
        Self { target: -15 }
    }
}
