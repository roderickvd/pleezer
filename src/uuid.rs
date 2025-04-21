//! UUID functionality with fast random generation.
//!
//! This module provides a wrapper around `uuid::Uuid` with additional functionality,
//! particularly focusing on fast UUID v4 generation using the `fastrand` crate.
//!
//! # Features
//! - Fast UUID v4 generation using `fastrand`
//! - Full compatibility with `uuid::Uuid` through `Deref`
//! - Implements common traits: `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`
//! - String parsing and formatting via `FromStr` and `Display`
//!
//! # Example
//! ```
//! use std::str::FromStr;
//!
//! // Generate a new UUID
//! let uuid = Uuid::fast_v4();
//!
//! // Convert to string
//! let uuid_string = uuid.to_string();
//!
//! // Parse from string
//! let parsed_uuid = Uuid::from_str(&uuid_string).unwrap();
//!
//! assert_eq!(uuid, parsed_uuid);
//! ```
//!
//! # Performance Note
//! While this implementation provides faster UUID generation using `fastrand`,
//! it should not be used in security-critical contexts where UUID predictability
//! must be prevented.

use crate::error::Error;
use std::{fmt, ops::Deref, str::FromStr};

/// A wrapper around `uuid::Uuid` that provides additional functionality.
///
/// This type implements `Deref` to `uuid::Uuid`, allowing transparent access to all
/// methods of the underlying UUID type.
///
/// # Example
/// ```
/// let uuid = Uuid::fast_v4();
/// let bytes = uuid.as_bytes(); // Accessing underlying uuid::Uuid method through Deref
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid(pub uuid::Uuid);

/// Provides transparent access to all methods of the underlying `uuid::Uuid` type.
impl Deref for Uuid {
    type Target = uuid::Uuid;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Uuid {
    /// Generates a new random UUID v4 using a fast random number generator.
    ///
    /// This method uses `fastrand` instead of the default random number generator
    /// for improved performance. While this generator is faster than cryptographically
    /// secure random number generators, it should not be used in security-sensitive
    /// contexts where UUID predictability must be prevented.
    ///
    /// # Returns
    /// A new randomly generated UUID wrapped in the `Uuid` type.
    ///
    /// # Example
    /// ```
    /// let uuid = Uuid::fast_v4();
    /// println!("{}", uuid); // Prints a UUID like "550e8400-e29b-41d4-a716-446655440000"
    /// ```
    #[must_use]
    #[inline]
    pub fn fast_v4() -> Self {
        let random_bytes = fastrand::u128(..).to_ne_bytes();
        let uuid = uuid::Builder::from_random_bytes(random_bytes).into_uuid();
        Self(uuid)
    }
}

/// Formats the UUID using the underlying `uuid::Uuid` Display implementation.
///
/// The UUID is formatted as a string of 32 hexadecimal digits with hyphens,
/// in the format: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
///
/// # Example
/// ```
/// let uuid = Uuid::fast_v4();
/// println!("{}", uuid); // e.g., "550e8400-e29b-41d4-a716-446655440000"
/// ```
impl fmt::Display for Uuid {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Parses a UUID string using the underlying `uuid::Uuid` `FromStr` implementation.
///
/// # Formats
/// Supports parsing UUIDs in these formats:
/// - Simple: `67e55044f3a340e6b5c0e090eb28b36`
/// - Hyphenated: `67e5504-4f3a-340e-6b5c-0e090eb28b36`
/// - Braced: `{67e55044-f3a3-40e6-b5c0-e090eb28b36}`
/// - Urn: `urn:uuid:67e55044-f3a3-40e6-b5c0-e090eb28b36`
///
/// # Errors
/// Returns a `uuid::Error` if the string is not a valid UUID.
///
/// # Example
/// ```
/// use std::str::FromStr;
///
/// let uuid = Uuid::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
/// ```
impl FromStr for Uuid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::from_str(s).map(Self).map_err(Into::into)
    }
}

/// Converts this `Uuid` into a `uuid::Uuid`.
///
/// This allows seamless conversion from our `Uuid` type to the underlying `uuid::Uuid` type.
///
/// # Example
/// ```
/// let our_uuid = Uuid::fast_v4();
/// let std_uuid: uuid::Uuid = our_uuid.into();
/// ```
impl From<Uuid> for uuid::Uuid {
    #[inline]
    fn from(value: Uuid) -> Self {
        *value
    }
}
