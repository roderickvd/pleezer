//! Live radio stream handling for Deezer's gateway API.
//!
//! Provides livestream-specific wrappers and types for:
//! * Stream URLs in multiple formats (AAC/MP3)
//! * Multiple bitrate options (32k, 64k, 96k, 192k)
//! * Station metadata (title, description, image)
//! * Country availability and status
//!
//! Livestreams have unique characteristics:
//! * Continuous streaming without duration
//! * Multiple quality/codec combinations
//! * Country-specific availability
//! * Station metadata instead of track info
//!
//! # Wire Format
//!
//! Response format:
//! ```json
//! {
//!     "LIVESTREAM_ID": "12345",
//!     "LIVESTREAM_TITLE": "Lorem Ipsum",
//!     "LIVESTREAM_DESCRIPTION": "",
//!     "LIVESTREAM_IMAGE_MD5": "7e3ccxxxxxxxxxxxxxxxxxxxxxxxxx03",
//!     "LIVESTREAM_IS_FINGERPRINTED": "0",
//!     "LIVESTREAM_URLS": {
//!         "data": {
//!             "96": {
//!                 "mp3": "https://example.com/stream/96.mp3"
//!             },
//!             "192": {
//!                 "mp3": "https://example.com/stream/192.mp3"
//!             },
//!             "64": {
//!                 "aac": "https://example.com/stream/64.aac"
//!             },
//!             "32": {
//!                 "aac": "https://example.com/stream/32.aac"
//!             }
//!         },
//!         "count": 4,
//!         "total": 4,
//!         "version": 1735116906,
//!         "filtered_count": 0
//!     },
//!     "LIVESTREAM_COUNTRY": "nl",
//!     "AVAILABLE": true,
//!     "__TYPE__": "livestream"
//! }
//! ```

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use super::{ListData, Method};
use crate::{protocol::Codec, track::TrackId};

/// Gateway method name for retrieving radio streams.
///
/// Returns stream information including:
/// * Station metadata
/// * Multiple quality streams
/// * Codec options
/// * Availability status
impl Method for LivestreamData {
    const METHOD: &'static str = "livestream.getData";
}

/// Wrapper for livestream data.
///
/// Contains the same track information as [`ListData`] but specifically
/// for podcast episodes. The wrapper allows specialized handling while
/// reusing the underlying data structure.
#[derive(Clone, PartialEq, Deserialize, Serialize, Debug)]
#[serde(transparent)]
#[expect(clippy::module_name_repetitions)]
pub struct LivestreamData(pub ListData);

/// Provides access to the underlying livestream data.
///
/// Allows transparent access to the livestream fields while maintaining
/// type safety for livestream-specific operations.
impl Deref for LivestreamData {
    type Target = ListData;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Request parameters for livestream data.
///
/// Used to request stream information with codec preferences.
///
/// # Example
///
/// ```rust
/// use deezer::gateway::{Request};
///
/// let request = Request {
///     livestream_id: 123456.into(),
///     supported_codecs: vec![Codec::AAC, Codec::MP3],
/// };
/// ```
#[serde_as]
#[derive(Clone, Eq, PartialEq, Serialize, Debug, Hash)]
pub struct Request {
    /// Live stream ID to fetch information for.
    pub livestream_id: TrackId,

    /// List of audio codecs supported by the client
    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub supported_codecs: Vec<Codec>,
}
