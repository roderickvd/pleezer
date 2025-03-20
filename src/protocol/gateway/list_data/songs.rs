//! Music track handling for Deezer's gateway API.
//!
//! Provides song-specific wrappers and types for:
//! * Track metadata (artist, album, title)
//! * Audio quality and encryption
//! * Volume normalization
//! * Content delivery
//!
//! Songs have specific features:
//! * Artist/album organization
//! * Volume normalization data
//! * Encrypted content delivery
//! * Quality selection
//!
//! # Wire Format
//!
//! Song response format:
//! ```json
//! {
//!     "SNG_ID": "123456",
//!     "ART_NAME": "Artist Name",
//!     "ALB_TITLE": "Album Title",
//!     "ALB_PICTURE": "album_cover_id",
//!     "DURATION": "180",
//!     "SNG_TITLE": "Track Title",
//!     "GAIN": "-1.3",
//!     "TRACK_TOKEN": "secret_token",
//!     "TRACK_TOKEN_EXPIRE": "1234567890"
//! }
//! ```
//!
//! Episode response format:
//! ```json
//! {
//!     "EPISODE_ID": "123456",
//!     "AVAILABLE": true,
//!     "DURATION": "1800",
//!     "EPISODE_TITLE": "Episode Title",
//!     "SHOW_NAME": "Podcast Name",
//!     "SHOW_ART_MD5": "cover_id",
//!     "TRACK_TOKEN": "secret_token",
//!     "TRACK_TOKEN_EXPIRE": "1234567890",
//!     "EPISODE_DIRECT_STREAM_URL": "https://..."
//! }
//! ```
//!
//! Livestream response format:
//! ```json
//! {
//!     "LIVESTREAM_ID": "123456",
//!     "LIVESTREAM_TITLE": "Station Name",
//!     "LIVESTREAM_IMAGE_MD5": "cover_id",
//!     "LIVESTREAM_URLS": {
//!         "data": {
//!             "64": {
//!                 "mp3": "https://...",
//!                 "aac": "https://..."
//!             }
//!         }
//!     },
//!     "AVAILABLE": true
//! }
//! ```

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::track::TrackId;

use super::{ListData, Method};

/// Gateway method name for retrieving songs.
///
/// Returns detailed track data including:
/// * Song metadata
/// * Album information
/// * Authentication tokens
/// * Quality options
/// * Volume normalization
impl Method for SongData {
    const METHOD: &'static str = "song.getListData";
}

/// Wrapper for song data.
///
/// Contains the same track information as [`ListData`] but specifically
/// for music songs. The wrapper allows specialized handling while
/// reusing the underlying data structure.
#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(transparent)]
pub struct SongData(pub ListData);

/// Provides access to the underlying song data.
///
/// Allows transparent access to the song fields while maintaining
/// type safety for song-specific operations.
impl Deref for SongData {
    type Target = ListData;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Request parameters for track list data.
///
/// Used to request information for multiple tracks in a single query.
/// Supports different content types through enum variants.
///
/// # Example
///
/// ```rust
/// use deezer::gateway::{Request, TrackId};
///
/// let request = Request::Songs {
///     song_ids: vec![123456.into(), 789012.into()],
/// };
/// ```
#[serde_as]
#[derive(Clone, Eq, PartialEq, Serialize, Debug, Hash)]
pub struct Request {
    /// List of track IDs to fetch information for.
    ///
    /// Each ID must be:
    /// * Non-zero
    /// * Either positive (Deezer tracks) or negative (user uploads)
    /// * Valid within Deezer's catalog
    #[serde(rename = "sng_ids")]
    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub song_ids: Vec<TrackId>,
}
