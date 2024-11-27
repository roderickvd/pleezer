use std::{ops::Deref, str::FromStr, time::SystemTime};

use serde::Deserialize;
use serde_with::{formats::Flexible, serde_as, DisplayFromStr, PickFirst, TimestampSeconds};
use url::Url;
use veil::Redact;

use crate::protocol::{self, connect::UserId};

use super::{Method, StringOrUnknown};

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

    #[serde(default)]
    #[serde(rename = "URL_MEDIA")]
    pub media_url: MediaUrl,

    #[serde(default)]
    #[serde(rename = "GAIN")]
    pub gain: Gain,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct MediaUrl(pub Url);

impl Deref for MediaUrl {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<MediaUrl> for Url {
    fn from(url: MediaUrl) -> Self {
        url.0
    }
}

impl Default for MediaUrl {
    fn default() -> Self {
        let media_url = Url::from_str("https://media.deezer.com").expect("invalid media url");
        Self(media_url)
    }
}

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct User {
    #[serde(rename = "USER_ID")]
    #[serde_as(as = "PickFirst<(_, DisplayFromStr)>")]
    pub id: UserId,

    #[serde(default)]
    #[serde(rename = "BLOG_NAME")]
    pub name: StringOrUnknown,

    #[serde(rename = "OPTIONS")]
    pub options: Options,

    #[serde(default)]
    #[serde(rename = "AUDIO_SETTINGS")]
    pub audio_settings: AudioSettings,
}

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Redact, Hash)]
pub struct Options {
    #[redact]
    pub license_token: String,

    #[serde(default)]
    pub too_many_devices: bool,

    #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
    pub expiration_timestamp: SystemTime,
}

#[serde_as]
#[derive(Clone, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct AudioSettings {
    #[serde_as(as = "DisplayFromStr")]
    pub connected_device_streaming_preset: protocol::connect::AudioQuality,
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Gatekeeps {
    // disable_device_limitation: bool,
    pub remote_control: bool,
}

impl Default for Gatekeeps {
    fn default() -> Self {
        Self {
            remote_control: true,
        }
    }
}

#[serde_as]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct Gain {
    #[serde(default)]
    #[serde(rename = "TARGET")]
    #[serde_as(as = "PickFirst<(DisplayFromStr, _)>")]
    pub target: i64,
}

impl Default for Gain {
    fn default() -> Self {
        Self { target: -15 }
    }
}
