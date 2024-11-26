use std::{fmt, time::SystemTime};

use serde::{Deserialize, Serialize};
use serde_with::{formats::Flexible, serde_as, DisplayFromStr, TimestampSeconds};
use url::Url;
use veil::Redact;

use super::connect::AudioQuality;

pub const DEFAULT_MEDIA_URL: &str = "https://media.deezer.com";

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Debug, Hash)]
pub struct Request {
    pub license_token: String,
    pub media: Vec<Media>,
    pub track_tokens: Vec<String>,
}

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Debug, Hash)]
pub struct Media {
    #[serde(rename = "type")]
    pub typ: Type,
    #[serde(rename = "formats")]
    pub cipher_formats: Vec<CipherFormat>,
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub enum Type {
    FULL,
    PREVIEW,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub struct CipherFormat {
    pub cipher: Cipher,
    pub format: Format,
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
#[expect(non_camel_case_types)]
pub enum Cipher {
    BF_CBC_STRIPE,
    NONE,
}

impl fmt::Display for Cipher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
#[expect(non_camel_case_types)]
#[repr(i64)]
pub enum Format {
    EXTERNAL = -1,
    FLAC = 9,
    MP3_64 = 10,
    MP3_128 = 1,
    MP3_320 = 3,
    MP3_MISC = 0,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<Format> for AudioQuality {
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

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub struct Response {
    pub data: Vec<Data>,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
#[serde(untagged)]
pub enum Data {
    Media { media: Vec<Medium> },
    Errors { errors: Vec<Error> },
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub struct Error {
    code: i64,
    message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub struct Medium {
    pub media_type: Type,
    pub cipher: CipherType,
    pub format: Format,
    pub sources: Vec<Source>,
    #[serde(rename = "nbf")]
    #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
    pub not_before: SystemTime,
    #[serde(rename = "exp")]
    #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
    pub expiry: SystemTime,
}

#[serde_as]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Debug, Hash)]
pub struct CipherType {
    #[serde(rename = "type")]
    pub typ: Cipher,
}

#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize, Redact, Hash)]
pub struct Source {
    #[serde_as(as = "DisplayFromStr")]
    #[redact]
    pub url: Url,
    pub provider: String,
}
