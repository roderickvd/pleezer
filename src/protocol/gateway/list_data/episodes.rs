//! Podcast episode handling for Deezer's gateway API.
//!
//! Provides episode-specific wrappers and types, including:
//! * Episode metadata (title, show, duration)
//! * External streaming URLs
//! * Availability status
//! * Show artwork
//!
//! Episodes differ from songs in several ways:
//! * Use direct streaming rather than encrypted downloads
//! * Include show/podcast metadata instead of artist/album
//! * Have region availability restrictions
//! * May be hosted outside Deezer's CDN
//!
//! # Wire Format
//!
//! Response format:
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

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use super::{ListData, Method};
use crate::track::TrackId;

/// Gateway method name for retrieving episodes.
///
/// This endpoint returns detailed episode data including:
/// * Episode metadata
/// * Show information
/// * Authentication tokens (if hosted on Deezer CDN)
/// * Streaming URLs
/// * Regional availability
impl Method for EpisodeData {
    const METHOD: &'static str = "episode.getListData";
}

/// Wrapper for episode data.
///
/// Contains the same track information as [`ListData`] but specifically
/// for podcast episodes. The wrapper allows specialized handling while
/// reusing the underlying data structure.
#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(transparent)]
pub struct EpisodeData(pub ListData);

/// Provides access to the underlying episode data.
///
/// Allows transparent access to the episode fields while maintaining
/// type safety for episode-specific operations.
impl Deref for EpisodeData {
    type Target = ListData;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Request parameters for episode list data.
///
/// Used to request information for multiple episodes in a single query.
/// Episodes must be available in the user's region to be retrieved.
///
/// # Example
///
/// ```rust
/// use deezer::gateway::{Request, TrackId};
///
/// let request = Request {
///     episode_ids: vec![123456.into(), 789012.into()],
/// };
/// ```
#[serde_as]
#[derive(Clone, Eq, PartialEq, Serialize, Debug, Hash)]
pub struct Request {
    /// List of episode IDs to fetch information for.
    ///
    /// Each ID must be:
    /// * Non-zero
    /// * Valid within Deezer's catalog
    /// * Available in user's region
    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub episode_ids: Vec<TrackId>,
}
