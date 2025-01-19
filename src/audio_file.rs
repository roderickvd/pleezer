//! Provides the `AudioFile` abstraction for handling audio stream playback.
//!
//! This module implements a unified interface for both encrypted and unencrypted audio files,
//! abstracting away the complexity of handling different stream types while providing
//! necessary traits for media playback.
//!
//! # Examples
//!
//! ```no_run
//! use pleezer::audio_file::AudioFile;
//! use std::io::{Read, Seek, SeekFrom};
//!
//! // Create audio file, handling potential errors
//! let mut audio = AudioFile::try_from_download(&track, download)?;
//!
//! // Check if seeking is supported
//! if audio.is_seekable() {
//!     audio.seek(SeekFrom::Start(1000))?;
//! }
//!
//! // Read data, handling I/O errors
//! let mut buf = vec![0; 1024];
//! match audio.read(&mut buf) {
//!     Ok(n) => println!("Read {n} bytes"),
//!     Err(e) => eprintln!("Read error: {e}"),
//! }
//! ```

use std::io::{BufReader, Read, Seek};

use stream_download::{storage::StorageProvider, StreamDownload};
use symphonia::core::io::MediaSource;

use crate::{decrypt::Decrypt, error::Result, track::Track};

/// Combines Read and Seek traits for audio stream handling.
///
/// This trait requires thread-safety (Send + Sync) to enable:
/// * Concurrent playback and downloading
/// * Safe sharing between threads
/// * Integration with async runtimes
pub trait ReadSeek: Read + Seek + Send + Sync {}

/// Blanket implementation for any type that implements both Read and Seek
impl<T: Read + Seek + Send + Sync> ReadSeek for T {}

/// Default buffer size for unencrypted audio stream reads (32 KiB).
///
/// This size is chosen to match Symphonia's read pattern, which reads
/// sequentially in increasing chunks up to 32 KiB.
///
/// Note: Encrypted streams use their own 2 KiB buffer through the `Decrypt` implementation.
pub const BUFFER_LEN: usize = 32 * 1024;

/// Represents an audio file stream that can be either encrypted or unencrypted.
///
/// `AudioFile` provides a unified interface for handling audio streams, automatically
/// managing encryption/decryption when needed while maintaining direct pass-through
/// for unencrypted content for optimal performance.
///
/// # Performance
///
/// The implementation uses different buffering strategies depending on the content:
/// * Unencrypted files use a 32 KiB buffer optimized for sequential audio reads
/// * Encrypted files use the `Decrypt` implementation with a 2 KiB buffer
pub struct AudioFile {
    /// The underlying stream implementation, either a direct stream or a decryptor
    inner: Box<dyn ReadSeek>,

    /// Indicates if seeking operations are supported (false for livestreams)
    is_seekable: bool,

    /// The total size of the audio file in bytes, if known
    byte_len: Option<u64>,
}

impl AudioFile {
    /// Creates a new `AudioFile` from a track and its download stream.
    ///
    /// This method automatically determines whether decryption is needed and sets up
    /// the appropriate stream handler:
    /// * For encrypted tracks: wraps the download in a `Decrypt` handler (2 KiB buffer)
    /// * For unencrypted tracks: uses buffered I/O with a 32 KiB buffer
    ///
    /// # Arguments
    ///
    /// * `track` - The track metadata containing encryption information
    /// * `download` - The underlying download stream
    ///
    /// # Type Parameters
    ///
    /// * `P` - The storage provider type implementing `StorageProvider`
    ///
    /// # Returns
    ///
    /// A new `AudioFile` configured for the track
    ///
    /// # Errors
    ///
    /// * `Error::Unimplemented` - Track uses unsupported encryption
    /// * `Error::PermissionDenied` - Decryption key not available
    /// * `Error::InvalidData` - Failed to create decryptor
    /// * Standard I/O errors from stream setup
    pub fn try_from_download<P>(track: &Track, download: StreamDownload<P>) -> Result<Self>
    where
        P: StorageProvider + Sync + 'static,
        P::Reader: Sync,
    {
        let is_seekable = !track.is_livestream();
        let byte_len = track.file_size();

        let result = if track.is_encrypted() {
            let decryptor = Decrypt::new(track, download)?;
            Self {
                inner: Box::new(decryptor),
                is_seekable,
                byte_len,
            }
        } else {
            let buffered = BufReader::with_capacity(BUFFER_LEN, download);
            Self {
                inner: Box::new(buffered),
                is_seekable,
                byte_len,
            }
        };

        Ok(result)
    }
}

/// Implements reading from the audio stream.
///
/// This implementation delegates all read operations directly to the underlying stream,
/// whether it's a decrypted stream or raw download stream, providing transparent
/// handling of encrypted and unencrypted content.
///
/// # Arguments
///
/// * `buf` - Buffer to read data into
///
/// # Returns
///
/// Number of bytes read, or 0 at end of stream
///
/// # Errors
///
/// Propagates errors from the underlying stream:
/// * `InvalidInput` - Buffer position invalid
/// * `InvalidData` - Decryption failed
/// * Standard I/O errors
impl Read for AudioFile {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

/// Implements seeking within the audio stream.
///
/// This implementation delegates all seek operations directly to the underlying stream.
/// Note that seeking may not be available for livestreams, which can be checked via
/// the `is_seekable()` method.
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
/// Propagates errors from the underlying stream:
/// * `InvalidInput` - Invalid seek position
/// * `UnexpectedEof` - Seek beyond end of file
/// * `Unsupported` - Seeking from end with unknown size
impl Seek for AudioFile {
    #[inline]
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Implements the `MediaSource` trait required by Symphonia for media playback.
///
/// This implementation provides metadata about the stream's capabilities and properties:
/// - Seekability: determined by whether the track is a livestream
/// - Byte length: provided if known from the track metadata
impl MediaSource for AudioFile {
    /// Returns whether seeking is supported in this audio stream.
    ///
    /// # Returns
    /// * `true` for normal audio files
    /// * `false` for livestreams
    #[inline]
    fn is_seekable(&self) -> bool {
        self.is_seekable
    }

    /// Returns the total size of the audio stream in bytes, if known.
    ///
    /// # Returns
    /// * `Some(u64)` - The size in bytes if known
    /// * `None` - If the size is unknown (e.g., for livestreams)
    #[inline]
    fn byte_len(&self) -> Option<u64> {
        self.byte_len
    }
}
