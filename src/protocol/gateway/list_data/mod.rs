//! Core types and functionality for Deezer content listings.
//!
//! This module provides shared data structures and traits for handling:
//! * Songs - Regular music tracks
//! * Episodes - Podcast episodes
//! * Livestreams - Radio stations
//!
//! Content types share common traits but have specialized handling through
//! type-specific wrappers in submodules:
//! * [`songs`] - Music track handling
//! * [`episodes`] - Podcast episode handling
//! * [`livestream`] - Radio stream handling
//!
//! # Content Types
//!
//! While all content types share basic metadata like IDs and titles,
//! they have unique characteristics:
//!
//! * Songs
//!   - Artist/album metadata
//!   - Volume normalization
//!   - Encrypted content
//!
//! * Episodes
//!   - Show/podcast metadata
//!   - External streaming URLs
//!   - Availability flags
//!
//! * Livestreams
//!   - Multiple quality streams
//!   - Codec selection
//!   - No duration/progress
//!
//! # Wire Format
//!
//! Each content type has its own response format:
//!
//! ## Songs
//! ```json
//! {
//!     "SNG_ID": "123456",
//!     "ART_NAME": "Artist Name",
//!     "ALB_TITLE": "Album Title",
//!     ...
//! }
//! ```
//!
//! ## Episodes
//! ```json
//! {
//!     "EPISODE_ID": "123456",
//!     "EPISODE_TITLE": "Episode Title",
//!     "SHOW_NAME": "Show Name",
//!     ...
//! }
//! ```
//!
//! ## Livestreams
//! ```json
//! {
//!     "LIVESTREAM_ID": "123456",
//!     "LIVESTREAM_TITLE": "Station Name",
//!     "LIVESTREAM_URLS": { ... },
//!     ...
//! }
//! ```

pub mod episodes;
pub mod livestream;
pub mod songs;

pub use episodes::EpisodeData;
pub use livestream::LivestreamData;
pub use songs::SongData;

use std::{
    collections::HashMap,
    ops::Deref,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use serde_with::{
    DefaultOnError, DisplayFromStr, DurationSeconds, PickFirst, TimestampSeconds,
    formats::Flexible, serde_as,
};
use url::Url;
use veil::Redact;

use crate::track::TrackId;

use super::Method;

/// Collection of track list data responses.
///
/// Contains a list of tracks that can be:
/// * Songs from the Deezer catalog
/// * User-uploaded songs
/// * Podcast episodes
/// * Live radio streams
pub type Queue = Vec<ListData>;

/// Detailed track information from Deezer's gateway.
///
/// Contains metadata and authentication information needed to play content:
/// * Unique identifiers
/// * Titles and artist/show information
/// * Media assets (covers)
/// * Playback details (duration, gain)
/// * Authentication tokens
/// * Availability information
///
/// Supports multiple content types through enum variants:
/// * Songs - Regular music tracks
/// * Episodes - Podcast episodes
/// * Livestreams - Radio stations
///
/// # Fields
///
/// * `track_id` - Unique track identifier
/// * `artist` - Artist name
/// * `album_title` - Album name
/// * `album_cover` - Album artwork identifier
/// * `duration` - Track length
/// * `title` - Track name
/// * `gain` - Volume normalization value
/// * `track_token` - Authentication token for playback
/// * `expiry` - Token expiration timestamp
///
/// # Example
///
/// ```rust
/// use deezer::gateway::{ListData, Response};
///
/// let response: Response<ListData> = /* gateway response */;
/// if let Some(track) = response.first() {
///     println!("{} by {}", track.title, track.artist);
///     println!("Token expires: {:?}", track.expiry);
/// }
/// ```
#[serde_as]
#[derive(Clone, PartialEq, Deserialize, Serialize, Redact)]
#[serde(tag = "__TYPE__")]
pub enum ListData {
    /// Regular music track
    #[serde(rename = "song")]
    Song {
        /// Unique song identifier.
        ///
        /// This ID can be:
        /// * Positive - Regular Deezer songs
        /// * Negative - User-uploaded songs
        #[serde(rename = "SNG_ID")]
        #[serde_as(as = "PickFirst<(DisplayFromStr, _)>")]
        id: TrackId,

        /// Artist name.
        ///
        /// For songs with multiple artists, this contains only the main artist.
        #[serde(default)]
        #[serde(rename = "ART_NAME")]
        artist: String,

        /// Album title.
        ///
        /// For singles or EPs, this might be the same as the song title.
        #[serde(default)]
        #[serde(rename = "ALB_TITLE")]
        album_title: String,

        /// Album cover identifier.
        ///
        /// When available, this ID can be used to construct image URLs:
        /// ```text
        /// https://cdn-images.dzcdn.net/images/cover/{album_cover}/{resolution}x{resolution}.{format}
        /// ```
        /// Where:
        /// * `resolution` is the desired size in pixels (up to 1920)
        /// * `format` is either:
        ///   - `jpg` for smaller file size
        ///   - `png` for higher quality
        ///
        /// Deezer's default format is 500x500.jpg
        ///
        /// Defaults to an empty string when no cover is available.
        #[serde(default)]
        #[serde(rename = "ALB_PICTURE")]
        album_cover: String,

        /// Song duration.
        ///
        /// The actual playback length of the song, parsed from seconds.
        /// Used for progress calculation and UI display.
        /// Defaults to zero duration if not provided or invalid.
        #[serde(default)]
        #[serde(rename = "DURATION")]
        #[serde_as(as = "DurationSeconds<String, Flexible>")]
        duration: Duration,

        /// Song title.
        ///
        /// This is the main display title of the song.
        #[serde(default)]
        #[serde(rename = "SNG_TITLE")]
        title: String,

        /// Song's average loudness in decibels (dB).
        ///
        /// Used to calculate volume normalization. May be absent if
        /// loudness data isn't available.
        ///
        /// Negative values indicate quieter songs (typical range: -20 to 0 dB).
        #[serde(rename = "GAIN")]
        #[serde_as(as = "Option<DisplayFromStr>")]
        gain: Option<f64>,

        /// Authentication token for song playback.
        ///
        /// This token is required to access the song's media content and:
        /// * Is unique per track
        /// * Has a limited validity period
        /// * Should be kept secure
        #[serde(rename = "TRACK_TOKEN")]
        #[redact]
        track_token: String,

        /// Token expiration timestamp.
        ///
        /// The time at which the `track_token` becomes invalid.
        /// New tokens should be requested after expiration.
        #[serde(rename = "TRACK_TOKEN_EXPIRE")]
        #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
        expiry: SystemTime,

        /// Fallback track data when primary track is unavailable.
        ///
        /// Some songs may have an alternative version available when the primary
        /// version cannot be accessed. This commonly occurs with:
        /// * Region-restricted content
        /// * Alternative recordings/mixes
        /// * Re-released versions
        ///
        /// When a fallback is used, the track's metadata is swapped with the
        /// fallback version's metadata.
        #[serde(rename = "FALLBACK")]
        fallback: Option<Box<Self>>,
    },

    /// Podcast episode
    #[serde(rename = "episode")]
    Episode {
        /// Unique episode identifier.
        #[serde(rename = "EPISODE_ID")]
        #[serde_as(as = "PickFirst<(DisplayFromStr, _)>")]
        id: TrackId,

        /// Whether the episode is available in the user's region
        #[serde(rename = "AVAILABLE")]
        #[serde(default)]
        available: bool,

        /// Episode duration.
        ///
        /// The actual playback length of the episode, parsed from seconds.
        /// Used for progress calculation and UI display.
        /// Defaults to zero duration if not provided or invalid.        #[serde(default)]
        #[serde(rename = "DURATION")]
        #[serde_as(as = "DurationSeconds<String, Flexible>")]
        duration: Duration,

        /// Direct streaming URL for the episode.
        ///
        /// Unlike songs which require token-based downloads,
        /// episodes are streamed directly from this URL.
        #[serde(rename = "EPISODE_DIRECT_STREAM_URL")]
        external_url: Option<Url>,

        /// Episode title.
        ///
        /// This is the main display title of the episode.
        #[serde(default)]
        #[serde(rename = "EPISODE_TITLE")]
        title: String,

        /// Whether this is an external stream.
        ///
        /// True for episodes hosted outside Deezer's CDN.
        #[serde(default)]
        #[serde(rename = "SHOW_IS_DIRECT_STREAM")]
        #[serde(deserialize_with = "bool_from_string")]
        external: bool,

        /// Show name.
        ///
        /// The name of the podcast this episode belongs to.
        /// For shows with multiple hosts, this contains only the main host.
        #[serde(default)]
        #[serde(rename = "SHOW_NAME")]
        podcast_title: String,

        /// Podcast cover identifier.
        ///
        /// When available, this ID can be used to construct image URLs:
        /// ```text
        /// https://cdn-images.dzcdn.net/images/talk/{podcast_art}/{resolution}x{resolution}.{format}
        /// ```
        /// Where:
        /// * `resolution` is the desired size in pixels (up to 1920)
        /// * `format` is either:
        ///   - `jpg` for smaller file size
        ///   - `png` for higher quality
        ///
        /// Deezer's default format is 500x500.jpg
        ///
        /// Defaults to an empty string when no cover is available.
        #[serde(default)]
        #[serde(rename = "SHOW_ART_MD5")]
        podcast_art: String,

        /// Authentication token for podcast playback from Deezer's CDN.
        ///
        /// This token is required to access the podcast's media content and:
        /// * Is unique per episode
        /// * Has a limited validity period
        /// * Should be kept secure
        #[serde(rename = "TRACK_TOKEN")]
        #[redact]
        track_token: String,

        /// Token expiration timestamp.
        ///
        /// The time at which the `track_token` becomes invalid.
        /// New tokens should be requested after expiration.
        #[serde(rename = "TRACK_TOKEN_EXPIRE")]
        #[serde_as(as = "TimestampSeconds<i64, Flexible>")]
        expiry: SystemTime,
    },

    /// Live radio stream
    #[serde(rename = "livestream")]
    Livestream {
        /// Unique live stream identifier.
        #[serde(rename = "LIVESTREAM_ID")]
        #[serde_as(as = "PickFirst<(_, DisplayFromStr)>")]
        id: TrackId,

        /// Live stream title.
        ///
        /// The name of the radio station.
        #[serde(default)]
        #[serde(rename = "LIVESTREAM_TITLE")]
        title: String,

        /// Live stream art identifier.
        ///
        /// When available, this ID can be used to construct image URLs:
        /// ```text
        /// https://cdn-images.dzcdn.net/images/cover/{cover_id}/{resolution}x{resolution}.{format}
        /// ```
        /// Where:
        /// * `resolution` is the desired size in pixels (up to 1920)
        /// * `format` is either:
        ///   - `jpg` for smaller file size
        ///   - `png` for higher quality
        ///
        /// Deezer's default format is 500x500.jpg
        ///
        /// Defaults to an empty string when no cover is available.
        #[serde(default)]
        #[serde(rename = "LIVESTREAM_IMAGE_MD5")]
        live_stream_art: String,

        /// Live stream URLs.
        ///
        /// Contains a list of available stream URLs for different bitrates and codecs.
        #[serde(rename = "LIVESTREAM_URLS")]
        #[serde_as(deserialize_as = "DefaultOnError")]
        external_urls: LivestreamUrls,

        /// Live stream availability status.
        ///
        /// Indicates whether the live stream is currently available for playback.
        #[serde(rename = "AVAILABLE")]
        #[serde(default)]
        available: bool,
    },
}

/// Converts string "1"/"0" to boolean values.
///
/// Used for fields that are boolean in logic but transmitted as strings:
/// * "1" -> true
/// * "0" -> false
/// * anything else -> error
///
/// Used primarily for availability and stream type flags.
fn bool_from_string<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.as_str() {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(serde::de::Error::custom("invalid boolean string")),
    }
}

impl ListData {
    /// Returns the type of this track.
    ///
    /// Returns a string identifier for the content type:
    /// * "song" - Regular music track
    /// * "episode" - Podcast episode
    /// * "livestream" - Radio station
    #[must_use]
    #[inline]
    pub const fn typ(&self) -> &'static str {
        match self {
            ListData::Song { .. } => "song",
            ListData::Episode { .. } => "episode",
            ListData::Livestream { .. } => "livestream",
        }
    }

    /// Returns the unique identifier for this content.
    ///
    /// IDs can be:
    /// * Positive - Regular Deezer content
    /// * Negative - User uploaded songs
    #[must_use]
    #[inline]
    pub fn id(&self) -> TrackId {
        match self {
            ListData::Song { id, .. }
            | ListData::Episode { id, .. }
            | ListData::Livestream { id, .. } => *id,
        }
    }

    /// Returns the title of this track.
    ///
    /// Returns None for livestreams which only have a station name.
    #[must_use]
    #[inline]
    pub fn title(&self) -> Option<&str> {
        match self {
            ListData::Song { title, .. } | ListData::Episode { title, .. } => Some(title.as_str()),
            ListData::Livestream { .. } => None,
        }
    }

    /// Returns the artist of this track.
    ///
    /// Returns:
    /// * Song artist for songs
    /// * Podcast name for episodes
    /// * Station name for livestreams
    #[must_use]
    #[inline]
    pub fn artist(&self) -> &str {
        match self {
            ListData::Song { artist, .. } => artist.as_str(),
            ListData::Episode { podcast_title, .. } => podcast_title.as_str(),
            ListData::Livestream { title, .. } => title.as_str(),
        }
    }

    /// Returns the cover art identifier for this track.
    ///
    /// Returns:
    /// * Album cover ID for songs
    /// * Podcast artwork ID for episodes
    /// * Station logo ID for livestreams
    #[must_use]
    #[inline]
    pub fn cover_id(&self) -> &str {
        match self {
            ListData::Song { album_cover, .. } => album_cover,
            ListData::Episode { podcast_art, .. } => podcast_art,
            ListData::Livestream {
                live_stream_art, ..
            } => live_stream_art,
        }
    }

    /// Returns the duration of this track.
    ///
    /// Returns:
    /// * Track duration for songs
    /// * Episode duration for podcasts
    /// * None for livestreams
    #[must_use]
    #[inline]
    pub fn duration(&self) -> Option<Duration> {
        match self {
            ListData::Song { duration, .. } | ListData::Episode { duration, .. } => Some(*duration),
            ListData::Livestream { .. } => None,
        }
    }

    /// Returns the authentication token if required.
    ///
    /// Returns:
    /// * Songs - Track token for encrypted content
    /// * Episodes - Track token for Deezer CDN
    /// * Livestreams - None (uses direct URLs)
    #[must_use]
    #[inline]
    pub fn track_token(&self) -> Option<&str> {
        match self {
            ListData::Song { track_token, .. } | ListData::Episode { track_token, .. } => {
                Some(track_token)
            }
            ListData::Livestream { .. } => None,
        }
    }

    /// Returns the expiration time for access token.
    ///
    /// Returns:
    /// * Songs - Track token expiry
    /// * Episodes - Track token expiry
    /// * Livestreams - None (no token needed)
    #[must_use]
    #[inline]
    pub fn expiry(&self) -> Option<SystemTime> {
        match self {
            ListData::Song { expiry, .. } | ListData::Episode { expiry, .. } => Some(*expiry),
            ListData::Livestream { .. } => None,
        }
    }
}

/// Key-value mapping of bitrates to codec URLs.
///
/// Keys are bitrate strings (e.g., "64", "128")
/// Values are codec-specific URLs for that bitrate
pub type LivestreamUrl = HashMap<String, CodecUrl>;

/// Quality-based stream URL mapping.
///
/// Maps bitrate strings to codec URLs:
/// ```json
/// {
///     "64": { "aac": "...", "mp3": "..." },
///     "128": { "aac": "...", "mp3": "..." }
/// }
/// ```
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LivestreamUrls {
    /// Quality-based stream URL mapping.
    pub data: LivestreamUrl,
}

/// Provides access to the underlying URL mapping.
///
/// Allows direct access to quality->codec->URL mappings while
/// maintaining type safety for livestream operations.
impl Deref for LivestreamUrls {
    type Target = LivestreamUrl;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl LivestreamUrls {
    /// Returns a Vec of (bitrate, `CodecUrl`) pairs sorted by bitrate (ascending order)
    #[must_use]
    pub fn sort_by_bitrate(&self) -> Vec<(usize, CodecUrl)> {
        let mut entries: Vec<_> = self
            .data
            .iter()
            .filter_map(|(bitrate, codec_url)| {
                // Parse bitrate string to usize
                bitrate.parse::<usize>().ok().map(|num| (num, codec_url))
            })
            .collect();

        // Sort by bitrate number
        entries.sort_by_key(|(bitrate, _)| *bitrate);

        // Create final Vec with sorted entries
        entries
            .into_iter()
            .map(|(bitrate, codec_url)| (bitrate, codec_url.clone()))
            .collect()
    }
}

/// URLs for different audio codecs of a livestream.
///
/// Provides access to stream URLs for different audio formats:
/// * AAC - Advanced Audio Coding
/// * MP3 - MPEG Layer-3
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash, Redact)]
#[redact(all)]
pub struct CodecUrl {
    /// URL for AAC stream if available
    pub aac: Option<Url>,
    /// URL for MP3 stream if available
    pub mp3: Option<Url>,
}
