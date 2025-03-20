//! Gateway API types and functionality for Deezer's web services.
//!
//! This module provides type-safe interfaces to Deezer's gateway API endpoints,
//! handling:
//! * Authentication tokens ([`arl`])
//! * User data and settings ([`user_data`])
//! * Content listings ([`list_data`])
//! * Radio stations ([`user_radio`])
//!
//! Supports multiple content types:
//! * Songs - Regular music tracks
//! * Episodes - Podcast episodes
//! * Livestreams - Radio stations (future)
//!
//! # Number Handling
//!
//! All numeric values are stored as 64-bit integers because the JSON protocol
//! doesn't distinguish between number sizes. This ensures safe handling of all
//! possible values from the API.
//!
//! # Response Types
//!
//! The API returns two types of responses:
//! * Paginated lists ([`Response::Paginated`])
//! * Simple results ([`Response::Unpaginated`])
//!
//! # Example
//!
//! ```rust
//! use deezer::gateway::{Response, UserData};
//!
//! // Parse a user data response
//! let response: Response<UserData> = serde_json::from_str(json)?;
//!
//! // Access the first result
//! if let Some(user) = response.first() {
//!     println!("User: {}", user.name);
//! }
//! ```

pub mod arl;
pub mod list_data;
pub mod user_data;
pub mod user_radio;

pub use arl::Arl;
pub use list_data::{
    EpisodeData, ListData, LivestreamData, LivestreamUrl, LivestreamUrls, Queue, SongData,
    episodes, livestream, songs,
};
pub use user_data::{MediaUrl, UserData};
pub use user_radio::UserRadio;

use std::collections::HashMap;

use serde::Deserialize;
use serde_with::serde_as;

/// Defines a gateway API method identifier.
///
/// Each type implementing this trait represents a specific Deezer gateway API
/// endpoint, identified by a method name string.
///
/// # Examples
///
/// ```rust
/// use deezer::gateway::{Method, Arl};
///
/// // ARL endpoint
/// assert_eq!(Arl::METHOD, "user.getArl");
///
/// // Generic function using method name
/// fn call_api<T: Method>(params: &str) {
///     println!("Calling {}: {}", T::METHOD, params);
/// }
/// ```
pub trait Method {
    /// The gateway API method name.
    ///
    /// This constant identifies the specific API endpoint, using Deezer's
    /// dot-notation format (e.g., "user.getArl").
    const METHOD: &'static str;
}

/// Response from a Deezer gateway API endpoint.
///
/// Can contain either:
/// * Regular content (songs, user uploads)
/// * Episodes (podcasts)
/// * Livestreams (radio)
///
/// The response format varies by content type but always includes:
/// * Error information
/// * Results array or pagination
///
/// # Response Formats
///
/// Paginated format:
/// ```json
/// {
///     "error": {},
///     "results": {
///         "data": [...],
///         "count": 10,
///         "total": 100,
///         "filtered_count": 10
///     }
/// }
/// ```
///
/// Unpaginated format:
/// ```json
/// {
///     "error": {},
///     "results": [...]  // Direct array or single item
/// }
/// ```
#[serde_as]
#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(untagged)]
pub enum Response<T> {
    /// Paginated response with result counts
    Paginated {
        /// API status information
        #[serde_as(as = "serde_with::Seq<(_, _)>")]
        error: HashMap<String, serde_json::Value>,
        /// Paginated result set
        results: Paginated<T>,
    },

    /// Direct response with results array
    Unpaginated {
        /// API status information
        #[serde_as(as = "serde_with::Seq<(_, _)>")]
        error: HashMap<String, serde_json::Value>,
        /// Result items (single item or array)
        #[serde_as(as = "serde_with::OneOrMany<_>")]
        results: Vec<T>,
    },
}

impl<T> Response<T> {
    /// Returns the first result item, if any.
    ///
    /// Works consistently for both paginated and unpaginated responses.
    ///
    /// # Examples
    ///
    /// ```rust
    /// if let Some(item) = response.first() {
    ///     println!("First item: {:?}", item);
    /// }
    /// ```
    #[must_use]
    #[inline]
    pub fn first(&self) -> Option<&T> {
        self.all().first()
    }

    /// Returns all result items as a slice.
    ///
    /// Works consistently for both paginated and unpaginated responses.
    ///
    /// # Examples
    ///
    /// ```rust
    /// for item in response.all() {
    ///     println!("Item: {:?}", item);
    /// }
    /// ```
    #[must_use]
    #[inline]
    pub fn all(&self) -> &Vec<T> {
        match self {
            Self::Paginated { results, .. } => &results.data,
            Self::Unpaginated { results, .. } => results,
        }
    }
}

/// Converts episode responses into list data responses.
///
/// This allows episode data to be handled using the same infrastructure
/// as other content types while maintaining type safety for episode-specific
/// operations.
impl From<Response<EpisodeData>> for Response<ListData> {
    fn from(response: Response<EpisodeData>) -> Self {
        match response {
            Response::Paginated { error, results } => {
                let results = Paginated {
                    data: results.data.into_iter().map(|data| data.0).collect(),
                    count: results.count,
                    total: results.total,
                    filtered_count: results.filtered_count,
                };
                Response::Paginated { error, results }
            }
            Response::Unpaginated { error, results } => Response::Unpaginated {
                error,
                results: results.into_iter().map(|data| data.0).collect(),
            },
        }
    }
}

/// Converts episode responses into list data responses.
///
/// This allows episode data to be handled using the same infrastructure
/// as other content types while maintaining type safety for episode-specific
/// operations.
impl From<Response<SongData>> for Response<ListData> {
    fn from(response: Response<SongData>) -> Self {
        match response {
            Response::Paginated { error, results } => {
                let results = Paginated {
                    data: results.data.into_iter().map(|data| data.0).collect(),
                    count: results.count,
                    total: results.total,
                    filtered_count: results.filtered_count,
                };
                Response::Paginated { error, results }
            }
            Response::Unpaginated { error, results } => Response::Unpaginated {
                error,
                results: results.into_iter().map(|data| data.0).collect(),
            },
        }
    }
}

/// Converts livestream responses into list data responses.
///
/// This allows livestream data to be handled using the same infrastructure
/// as other content types while maintaining type safety for livestream-specific
/// operations.
impl From<Response<LivestreamData>> for Response<ListData> {
    fn from(response: Response<LivestreamData>) -> Self {
        match response {
            Response::Paginated { error, results } => {
                let results = Paginated {
                    data: results.data.into_iter().map(|data| data.0).collect(),
                    count: results.count,
                    total: results.total,
                    filtered_count: results.filtered_count,
                };
                Response::Paginated { error, results }
            }
            Response::Unpaginated { error, results } => Response::Unpaginated {
                error,
                results: results.into_iter().map(|data| data.0).collect(),
            },
        }
    }
}

/// Paginated result set from the Deezer gateway API.
///
/// Contains both the actual data items and metadata about the total
/// number of available items and filtering.
///
/// # Example Response
///
/// ```json
/// {
///     "data": [...],           // Actual items
///     "count": 10,            // Items in this page
///     "total": 100,           // Total available items
///     "filtered_count": 10    // Items matching filters
/// }
/// ```
#[derive(Clone, PartialEq, Deserialize, Debug)]
pub struct Paginated<T> {
    /// Items in this page of results
    pub data: Vec<T>,
    /// Number of items in this page
    pub count: u64,
    /// Total number of items available
    pub total: u64,
    /// Number of items matching applied filters
    pub filtered_count: u64,
}
