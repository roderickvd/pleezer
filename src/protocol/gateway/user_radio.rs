//! Deezer Flow personalized radio endpoint.
//!
//! This module handles fetching tracks for Deezer Flow, a personalized
//! radio stream that provides batches of recommended tracks based on user
//! preferences and listening history.
//!
//! # Wire Format
//!
//! Request:
//! ```json
//! {
//!     "user_id": "123456789"
//! }
//! ```
//!
//! Response contains a list of tracks in the same format as [`ListData`].
//!
//! # Example
//!
//! ```rust
//! use deezer::gateway::{Response, UserRadio, UserId};
//!
//! // Request Flow tracks
//! let request = Request {
//!     user_id: 123456789.into(),
//! };
//!
//! let response: Response<UserRadio> = /* gateway response */;
//! for track in response.all() {
//!     println!("Flow track: {} by {}", track.title, track.artist);
//! }
//! ```

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use super::{ListData, Method};
use crate::protocol::connect::UserId;

/// Gateway method name for retrieving Flow tracks.
///
/// Returns a batch of track recommendations based on the user's
/// preferences and listening history.
impl Method for UserRadio {
    const METHOD: &'static str = "radio.getUserRadio";
}

/// Wrapper for Flow radio track data.
///
/// Contains the same track information as [`ListData`] but specifically
/// for tracks provided by Deezer Flow. Each response contains multiple
/// recommended tracks.
#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(transparent)]
pub struct UserRadio(pub ListData);

/// Provides access to the underlying track data.
///
/// # Examples
///
/// ```rust
/// use deezer::gateway::{Response, UserRadio};
///
/// let response: Response<UserRadio> = /* gateway response */;
/// if let Some(track) = response.first() {
///     // Access track data directly
///     println!("{} by {}", track.title, track.artist);
/// }
/// ```
impl Deref for UserRadio {
    type Target = ListData;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Request parameters for Flow radio tracks.
///
/// Used to request the next track in a user's personalized Flow stream.
#[serde_as]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Debug, Hash)]
pub struct Request {
    /// User ID to get Flow recommendations for.
    ///
    /// Must be a valid Deezer user ID. The recommendations will be
    /// based on this user's preferences and listening history.
    #[serde_as(as = "DisplayFromStr")]
    pub user_id: UserId,
}
