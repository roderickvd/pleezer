//! Track decryption for Deezer's protected media content.
//!
//! This module provides buffered reading of encrypted Deezer tracks:
//! * Processes encrypted content in 2KB blocks
//! * Handles Blowfish CBC encryption with striping
//! * Manages decryption keys and block alignment
//!
//! # Encryption Format
//!
//! Deezer uses a striped encryption pattern:
//! * Content is divided into 2KB blocks
//! * Every third block is encrypted
//! * Encryption uses Blowfish in CBC mode
//! * A fixed IV is used
//! * The cipher is reset after each block
//!
//! # Security
//!
//! To comply with Deezer's Terms of Service:
//! * No decryption keys are included in this code
//! * Keys must be provided externally
//!
//! # Memory Management
//!
//! The implementation:
//! * Processes all content in 2KB blocks
//! * Maintains an internal buffer for current block
//! * Decrypts blocks as needed based on striping pattern
//!
//! # Examples
//!
//! ```rust
//! use pleezer::decrypt::{Decrypt, Key};
//! use std::io::{Read, Seek};
//!
//! // Create decryptor with track and reader
//! let mut decryptor = Decrypt::new(&track, reader)?;
//!
//! // Read content in blocks
//! let mut buffer = Vec::new();
//! decryptor.read_to_end(&mut buffer)?;
//! ```
//!
//! # Implementation Details
//!
//! The decryptor provides:
//! * Efficient buffered reading via `BufRead` trait
//! * Proper seeking support with block alignment
//! * Automatic buffer management

use std::{
    cell::OnceCell,
    io::{self, BufRead, Read, Seek, SeekFrom},
    ops::Deref,
    str::FromStr,
};

use blowfish::{cipher::BlockDecryptMut, cipher::KeyIvInit, Blowfish};
use cbc::cipher::block_padding::NoPadding;
use md5::{Digest, Md5};

use crate::{
    audio_file::ReadSeek,
    error::{Error, Result},
    protocol::media::Cipher,
    track::{Track, TrackId},
};

/// Block-based reader for encrypted Deezer tracks.
///
/// Handles encrypted tracks by:
/// * Reading content in 2KB blocks
/// * Decrypting blocks based on stripe pattern
/// * Maintaining proper block alignment during seeks
///
/// # Block Processing
///
/// Content is processed in 2KB blocks with:
/// * Every third block decrypted using Blowfish CBC
/// * Proper block alignment maintained during seeks
/// * Buffered reading for efficiency
///
/// # Supported Encryption
///
/// Currently supports:
/// * Blowfish CBC with striping (every third 2KB block)
pub struct Decrypt<R>
where
    R: ReadSeek,
{
    /// Source of encrypted data using temporary file storage.
    file: R,

    /// Total size of the track in bytes, if known.
    ///
    /// Used for seek operations, particularly for seeking from
    /// the end of the track.
    file_size: Option<u64>,

    /// Track-specific decryption key.
    ///
    /// Derived from the track ID and Deezer master key using
    /// `key_for_track_id()`.
    key: Key,

    /// Decrypted data buffer.
    ///
    /// Contains the current 2KB block (or smaller for the last block)
    /// of decrypted data.
    buffer: [u8; CBC_BLOCK_SIZE],

    /// Current length of valid data in the buffer.
    ///
    /// May be less than `CBC_BLOCK_SIZE`, especially for the last block.
    buffer_len: usize,

    /// Current position within the buffer.
    ///
    /// Tracks how many bytes have been consumed from the current buffer.
    pos: u64,

    /// Current block number being processed.
    ///
    /// Used to track position in the stream and determine which
    /// blocks need decryption (every third block when using
    /// `BF_CBC_STRIPE`).
    block: Option<u64>,
}

/// Length of decryption keys in bytes.
pub const KEY_LENGTH: usize = 16;

/// Raw key bytes.
pub type RawKey = [u8; KEY_LENGTH];

/// Validated decryption key.
///
/// Ensures keys are the correct length and format for use
/// with Blowfish decryption.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Key(RawKey);

/// Parses a string into a decryption key.
///
/// The string must be exactly 16 bytes long, as required by
/// Blowfish and Deezer's encryption format.
///
/// # Errors
///
/// Returns `Error::OutOfRange` if the string length isn't
/// exactly 16 bytes.
///
/// # Examples
///
/// ```rust
/// use pleezer::decrypt::Key;
///
/// // Valid 16-byte key
/// let key: Key = "1234567890123456".parse()?;
///
/// // Too short
/// assert!("12345".parse::<Key>().is_err());
///
/// // Too long
/// assert!("12345678901234567".parse::<Key>().is_err());
/// ```
impl FromStr for Key {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let len = s.len();
        if len != KEY_LENGTH {
            return Err(Error::out_of_range(format!(
                "key length is {len} but should be {KEY_LENGTH}",
            )));
        }

        let bytes = s.as_bytes();
        let mut key = [0; KEY_LENGTH];
        key.copy_from_slice(bytes);

        Ok(Self(key))
    }
}

/// Provides read-only access to the raw key bytes.
///
/// This allows using the key with cryptographic functions
/// that expect byte arrays while maintaining key encapsulation.
///
/// # Examples
///
/// ```rust
/// use pleezer::decrypt::Key;
///
/// let key: Key = "1234567890123456".parse()?;
///
/// // Access raw bytes
/// assert_eq!(&*key, b"1234567890123456");
///
/// // Use with crypto functions
/// let cipher = Blowfish::new_from_slice(&*key)?;
/// ```
impl Deref for Key {
    type Target = RawKey;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Fixed IV for CBC decryption.
const CBC_BF_IV: &[u8; 8] = b"\x00\x01\x02\x03\x04\x05\x06\x07";

/// Block size for encryption and buffering (2KB).
/// This matches Deezer's encryption block size and provides
/// efficient buffering for both encrypted and unencrypted content.
const CBC_BLOCK_SIZE: usize = 2 * 1024;

/// Striping pattern for encrypted blocks.
/// Every third block is encrypted, matching Deezer's format.
const CBC_STRIPE_COUNT: usize = 3;

/// Supported encryption methods.
const SUPPORTED_CIPHERS: [Cipher; 1] = [Cipher::BF_CBC_STRIPE];

thread_local! {
    /// Global decryption key, set once and used for all decryption.
    static BF_SECRET: OnceCell<Key> = const { OnceCell::new() };
}

/// Sets the global decryption key.
///
/// Must be called before any decryption operations.
/// Can only be set once - subsequent calls will fail.
///
/// # Arguments
/// * `secret` - Master decryption key
///
/// # Errors
/// * `Error::Unimplemented` - Key has already been set
pub fn set_bf_secret(secret: Key) -> Result<()> {
    BF_SECRET.with(|cell| {
        cell.set(secret)
            .map_err(|_| Error::unimplemented("decryption key already set"))
    })
}

/// Retrieves the global decryption key.
///
/// # Errors
///
/// Returns `Error::Unimplemented` if the key hasn't been set.
fn bf_secret() -> Result<Key> {
    BF_SECRET.with(|cell| {
        cell.get()
            .copied()
            .ok_or_else(|| Error::permission_denied("decryption key not set"))
    })
}

impl<R> Decrypt<R>
where
    R: ReadSeek,
{
    /// Creates a new decryption stream for an encrypted track.
    ///
    /// # Arguments
    /// * `track` - Track metadata including encryption information
    /// * `file` - Reader providing the encrypted data
    ///
    /// # Returns
    /// A new decryptor configured for the track's encryption method
    ///
    /// # Errors
    /// * `Error::InvalidArgument` - Track is not encrypted
    /// * `Error::Unimplemented` - Track uses unsupported encryption method
    /// * `Error::PermissionDenied` - Global decryption key not set
    /// * `Error::InvalidData` - Failed to generate track-specific key
    pub fn new(track: &Track, file: R) -> Result<Self>
    where
        R: ReadSeek,
    {
        if !track.is_encrypted() {
            return Err(Error::invalid_argument(format!("{track} is not encrypted")));
        }
        if !SUPPORTED_CIPHERS.contains(&track.cipher()) {
            return Err(Error::unimplemented(format!(
                "unsupported encryption algorithm {}",
                track.cipher()
            )));
        }

        // Calculate decryption key.
        let salt = bf_secret()?;
        let key = Self::key_for_track_id(track.id(), &salt);

        Ok(Self {
            file,
            file_size: track.file_size(),
            key,
            buffer: [0; CBC_BLOCK_SIZE],
            buffer_len: 0,
            pos: 0,
            block: None,
        })
    }

    /// Derives a track-specific decryption key.
    ///
    /// The key is generated using:
    /// 1. MD5 hash of the track ID
    /// 2. XOR with the master decryption key (salt)
    ///
    /// # Arguments
    ///
    /// * `track_id` - Unique identifier for the track
    /// * `salt` - Master decryption key
    ///
    /// # Returns
    ///
    /// A new `Key` specific to this track for decryption.
    #[must_use]
    pub fn key_for_track_id(track_id: TrackId, salt: &Key) -> Key {
        let track_hash = format!("{:x}", Md5::digest(track_id.to_string()));
        let track_hash = track_hash.as_bytes();

        let mut key = RawKey::default();
        for i in 0..KEY_LENGTH {
            key[i] = track_hash[i] ^ track_hash[i + KEY_LENGTH] ^ salt[i];
        }
        Key(key)
    }
}

/// Seeks within the encrypted stream.
///
/// The implementation handles:
/// * Block alignment for encrypted content
/// * Buffer management for decryption
/// * Position calculations with stripe pattern
///
/// Maintains:
/// * 2KB block boundaries
/// * Decryption of every third block
/// * Proper stripe pattern alignment
///
/// # Arguments
///
/// * `pos` - Seek position (Start/Current/End)
///
/// # Returns
///
/// New position in the stream
///
/// # Errors
///
/// * `InvalidInput` - Seeking to negative or overflowing position
/// * `UnexpectedEof` - Seeking beyond end of file
/// * `Unsupported` - Seeking from end with unknown file size
impl<R> Seek for Decrypt<R>
where
    R: ReadSeek,
{
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // TODO: DRY up error messages
        let target = match pos {
            SeekFrom::Start(pos) => pos,
            SeekFrom::End(pos) => {
                let file_size = self.file_size.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "cannot seek from end with unknown size",
                    )
                })?;
                file_size
                    .checked_add_signed(pos)
                    .and_then(|pos| pos.checked_sub(1))
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "invalid seek to negative or overflowing position",
                        )
                    })?
            }
            SeekFrom::Current(pos) => {
                let current = self
                    .block
                    .unwrap_or_default()
                    .checked_mul(CBC_BLOCK_SIZE as u64)
                    .and_then(|block| block.checked_add(self.pos))
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "invalid seek to negative or overflowing position",
                        )
                    })?;

                current.checked_add_signed(pos).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid seek to negative or overflowing position",
                    )
                })?
            }
        };

        if self.file_size.is_some_and(|size| target >= size) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "seek beyond end of file",
            ));
        }

        let block = target.checked_div(CBC_BLOCK_SIZE as u64).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "block calculation would be divide by zero",
            )
        })?;
        let offset = target.checked_rem(CBC_BLOCK_SIZE as u64).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "offset calculation would be divide by zero",
            )
        })?;

        // Only read new block if different from current
        if self.block.is_none_or(|current| current != block) {
            self.block = Some(block);
            self.file
                .seek(SeekFrom::Start(block * CBC_BLOCK_SIZE as u64))?;

            // Use `read_exact` when we're sure we have a full block
            if self.file_size.is_some_and(|size| {
                let remaining_bytes = size.saturating_sub(block * CBC_BLOCK_SIZE as u64);
                remaining_bytes >= CBC_BLOCK_SIZE as u64
            }) {
                // Full block expected, use `read_exact` for efficiency
                self.file.read_exact(&mut self.buffer)?;
                self.buffer_len = CBC_BLOCK_SIZE;
            } else {
                // Partial block or unknown size, use regular `read`
                self.buffer_len = self.file.read(&mut self.buffer)?;
            }

            let is_encrypted = block % CBC_STRIPE_COUNT as u64 == 0;
            let is_full_block = self.buffer_len == CBC_BLOCK_SIZE;

            if is_encrypted && is_full_block {
                let cipher = cbc::Decryptor::<Blowfish>::new_from_slices(&*self.key, CBC_BF_IV)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

                cipher
                    .decrypt_padded_mut::<NoPadding>(&mut self.buffer[..CBC_BLOCK_SIZE])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            }
        }

        self.pos = offset;
        Ok(target)
    }
}

/// Provides buffered reading of content.
///
/// The implementation:
/// * Uses a 2KB buffer for all content
/// * Automatically fills buffer when empty
/// * Handles block-based decryption where needed
///
/// # Examples
///
/// ```no_run
/// use std::io::BufRead;
///
/// let mut decryptor = // ... create decryptor
/// while let Ok(buffer) = decryptor.fill_buf() {
///     if buffer.is_empty() {
///         break;
///     }
///     // Process buffer...
///     decryptor.consume(buffer.len());
/// }
/// ```
impl<R> BufRead for Decrypt<R>
where
    R: ReadSeek,
{
    /// Returns a reference to the internal buffer.
    ///
    /// Fills the buffer if empty, handling decryption if needed.
    ///
    /// # Returns
    ///
    /// Reference to buffered data
    ///
    /// # Errors
    ///
    /// * `InvalidInput` - Buffer position would be out of bounds
    /// * `InvalidData` - Decryption failed
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buffer_len as u64 {
            // Fill buffer with next decrypted block
            let _ = self.stream_position()?;
        }

        let pos = usize::try_from(self.pos).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer position would be out of bounds",
            )
        })?;

        Ok(&self.buffer[pos..self.buffer_len])
    }

    /// Marks a number of bytes as consumed.
    ///
    /// # Arguments
    ///
    /// * `amt` - Number of bytes to mark as consumed
    #[inline]
    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos.saturating_add(amt as u64)).min(self.buffer_len as u64);
    }
}

/// Reads data from the buffered stream.
///
/// The implementation uses the internal buffering mechanism to:
/// * Minimize system calls
/// * Handle decryption when needed
/// * Provide consistent buffered reading
///
/// # Arguments
///
/// * `buf` - Destination buffer for read data
///
/// # Returns
///
/// Number of bytes read, or 0 at end of stream
///
/// # Errors
///
/// * `InvalidInput` - Buffer position would be out of bounds
/// * `InvalidData` - Decryption failed
/// * Standard I/O errors from underlying stream operations
impl<R> Read for Decrypt<R>
where
    R: ReadSeek,
{
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let amt = available.len().min(buf.len());
        buf[..amt].copy_from_slice(&available[..amt]);
        self.consume(amt);
        Ok(amt)
    }
}
