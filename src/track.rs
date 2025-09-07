//! Track management and playback preparation.
//!
//! This module handles Deezer track operations including:
//! * Track metadata management
//! * Media source retrieval
//! * Audio stream handling through `AudioFile`
//! * Download management
//! * Format detection and audio parameters
//!
//! # Audio Parameters
//!
//! Tracks provide audio configuration information:
//! * Sample rate (defaults to 44.1 kHz)
//! * Bits per sample (codec-dependent)
//! * Channel count (content-specific defaults):
//!   - Songs: Stereo (2 channels)
//!   - Episodes: Mono (1 channel)
//!   - Livestreams: Stereo (2 channels)
//!
//! # Audio Format Support
//!
//! Different content types support different formats:
//! * Songs (Deezer catalog):
//!   - MP3 (CBR at 64/128/320 kbps)
//!   - FLAC (lossless)
//! * Episodes (Podcasts):
//!   - MP3 (variable bitrate)
//!   - AAC (in ADTS or MP4 container)
//!   - WAV (uncompressed PCM)
//! * Livestreams:
//!   - AAC (in ADTS container)
//!   - MP3
//!
//! # Track Lifecycle
//!
//! 1. Creation
//!    * From gateway API response
//!    * Contains metadata and tokens
//!
//! 2. Media Source Resolution
//!    * Retrieves download URLs
//!    * Negotiates quality/format
//!    * Validates availability
//!
//! 3. Stream Preparation
//!    * Creates `AudioFile` abstraction
//!    * Handles encryption if needed
//!    * Manages download buffering
//!
//! 4. Playback Management
//!    * Tracks download progress
//!    * Manages buffer state
//!    * Enables seeking within buffered data
//!
//! # Quality Fallback
//!
//! When requested quality isn't available, the system attempts fallback in order:
//! * FLAC → MP3 320 → MP3 128 → MP3 64
//! * MP3 320 → MP3 128 → MP3 64
//! * MP3 128 → MP3 64
//!
//! # Integration
//!
//! Works with:
//! * [`player`](crate::player) - For playback management
//! * [`gateway`](crate::gateway) - For track metadata
//! * [`audio_file`](crate::audio_file) - For unified stream handling
//! * [`decrypt`](crate::decrypt) - For encrypted content
//!
//! # Example
//!
//! ```rust
//! use pleezer::track::Track;
//!
//! // Create track from gateway data
//! let mut track = Track::from(track_data);
//!
//! // Get media source with proper content type for track metadata
//! let request = client.json(media_url, track_data);
//! let response = client.execute(request).await?;
//!
//! // Start download
//! track.start_download(&client, &medium).await?;
//!
//! // Monitor progress
//! println!("Downloaded: {:?} of {:?}", track.buffered(), track.duration());
//! ```

use std::{
    fmt,
    num::NonZeroI64,
    ops::Deref,
    str::FromStr,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime},
};

use rodio::SampleRate;
use stream_download::{
    self, StreamDownload, StreamHandle, StreamPhase, StreamState, http::HttpStream,
    source::SourceStream, storage::StorageProvider,
};
use time::OffsetDateTime;
use url::Url;
use veil::Redact;

use crate::{
    audio_file::AudioFile,
    error::{Error, Result},
    http,
    protocol::{
        self, Codec,
        connect::AudioQuality,
        gateway::{self, LivestreamUrls},
        media::{self, Cipher, CipherFormat, Data, Format, Medium},
    },
    util::ToF32,
};

/// Default audio sample rate in Hz.
pub const DEFAULT_SAMPLE_RATE: SampleRate = 44_100;

/// Default number of bits per sample.
pub const DEFAULT_BITS_PER_SAMPLE: u32 = 16;

/// A unique identifier for a track.
///
/// * Positive IDs: Regular Deezer tracks
/// * Negative IDs: User-uploaded tracks
#[expect(clippy::module_name_repetitions)]
pub type TrackId = NonZeroI64;

/// Type of track content.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
#[expect(clippy::module_name_repetitions)]
pub enum TrackType {
    /// Regular music track from Deezer catalog or user upload
    #[default]
    Song,
    /// Podcast episode with external streaming
    Episode,
    /// Live radio station with multiple streams
    Livestream,
}

impl TrackType {
    /// Stereo audio channel count.
    const STEREO: u16 = 2;
    /// Mono audio channel count.
    const MONO: u16 = 1;

    /// Default number of audio channels for this track type.
    ///
    /// * Songs and livestreams use stereo (2 channels)
    /// * Episodes (podcasts) use mono (1 channel)
    ///
    /// These defaults match typical encoding settings for each content type.
    /// Actual channel count may differ based on source material.
    #[must_use]
    pub fn default_channels(&self) -> u16 {
        match self {
            Self::Song | Self::Livestream => Self::STEREO,
            Self::Episode => Self::MONO,
        }
    }
}

/// External streaming URL configuration.
///
/// Handles streaming URLs for non-standard content:
/// * `Direct` - Single stream URL for podcast episodes
/// * `WithQuality` - Multiple quality/codec options for livestreams
///
/// URLs are redacted in debug output for security.
#[derive(Clone, Redact, Eq, PartialEq)]
#[redact(all, variant)]
pub enum ExternalUrl {
    /// Direct streaming URL (for episodes)
    Direct(Url),
    /// Multiple quality streams (for livestreams)
    WithQuality(gateway::LivestreamUrls),
}

/// Display implementation for track type.
impl fmt::Display for TrackType {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Song => write!(f, "song"),
            Self::Episode => write!(f, "episode"),
            Self::Livestream => write!(f, "livestream"),
        }
    }
}

impl FromStr for TrackType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "song" => Ok(Self::Song),
            "episode" => Ok(Self::Episode),
            "livestream" => Ok(Self::Livestream),
            _ => Err(Error::invalid_argument(format!("unknown track type: {s}"))),
        }
    }
}

/// Represents a Deezer track with metadata and download/buffering state.
///
/// Combines track metadata (title, artist, etc) with:
/// * Audio parameters:
///   - Sample rate (defaults to 44.1 kHz)
///   - Bits per sample (codec-dependent)
///   - Channel count (content-specific defaults)
/// * Download management (progress tracking, cancellation)
/// * Buffer management (seek limits, progress tracking)
/// * Quality/format information (codecs, bitrates)
/// * Encryption configuration
///
/// Download and buffering states are kept separate to:
/// * Allow seeks within buffered data while download continues
/// * Prevent blocking seeks beyond buffered data
/// * Track both download progress and playable duration
///
/// # Example
///
/// ```rust
/// use pleezer::track::Track;
///
/// let track = Track::from(track_data);
/// println!("Track: {} by {}", track.title(), track.artist());
/// println!("Duration: {:?}", track.duration());
/// ```
#[derive(Debug)]
pub struct Track {
    /// Type of content (song, episode, or livestream)
    typ: TrackType,

    /// Unique identifier for the track
    id: TrackId,

    /// Authentication token for media access.
    /// None for livestreams or when using external URLs.
    token: Option<String>,

    /// Whether content is served from external source
    external: bool,

    /// External URL for direct streaming.
    /// Used by episodes and livestreams.
    external_url: Option<ExternalUrl>,

    /// Title of the content.
    /// None for livestreams which only have station name.
    title: Option<String>,

    /// Content creator:
    /// * Artist name for songs
    /// * Show name for episodes
    /// * Station name for livestreams
    artist: String,

    /// Album title. Only available for songs.
    album_title: Option<String>,

    /// Identifier for cover artwork:
    /// * Album art for songs
    /// * Show art for episodes
    /// * Station logo for livestreams
    cover_id: String,

    /// Replay gain value in decibels.
    /// Used for volume normalization if available.
    /// Only available for songs, but not all songs have this value.
    gain: Option<f32>,

    /// When this track's access token expires.
    /// After this time, new tokens must be requested.
    /// Not available for livestreams.
    expiry: Option<SystemTime>,

    /// Current audio quality setting.
    /// May be lower than requested if any higher quality was unavailable.
    quality: AudioQuality,

    /// Total duration of the track.
    /// Not available for livestreams.
    duration: Option<Duration>,

    /// Amount of audio data downloaded and available for playback.
    /// Protected by mutex for concurrent access from download task.
    buffered: Arc<Mutex<Option<Duration>>>,

    /// Total size of the audio file in bytes.
    /// Available only after download begins.
    /// Not available for livestreams.
    file_size: Option<u64>,

    /// Encryption cipher used for this track.
    /// `Cipher::NONE` represents unencrypted content.
    cipher: Cipher,

    /// Handle to active download if any.
    /// None if download hasn't started or was reset.
    handle: Option<StreamHandle>,

    /// Whether the track is available for download.
    /// Only available for podcasts and episodes.
    /// Songs have this always set to `true`.
    /// Note that the expiry time should be checked separately.
    available: bool,

    /// Audio bitrate in kbps if known.
    /// * For MP3: Constant bitrate from quality level
    /// * For FLAC: Variable bitrate calculated from file size
    /// * For livestreams: Bitrate from stream URL
    bitrate: Option<usize>,

    /// Audio codec used for this content.
    /// * For regular tracks: Determined by quality level
    /// * For episodes: Inferred from URL extension
    /// * For livestreams: Determined from stream URL
    codec: Option<Codec>,

    /// Sample rate of the audio track.
    /// Set by player after decoder initialization.
    pub sample_rate: Option<SampleRate>,

    /// Number of bits per sample in the track.
    /// Set by player after decoder initialization.
    pub bits_per_sample: Option<u32>,

    /// Number of audio channels in the track.
    /// Set by player after decoder initialization.
    pub channels: Option<u16>,

    /// Fallback track to use when primary track is unavailable.
    /// * Contains complete track metadata
    /// * Used for alternative versions of same song
    /// * Swapped with primary track when fallback is needed
    /// * Reset when switching to preserve download state
    fallback: Option<Box<Self>>,
}

/// Internal stream state for content download.
///
/// Combines:
/// * HTTP stream for downloading content
/// * Source URL for format/codec detection
///
/// Format detection rules:
/// * Songs: Determined by quality level (MP3 or FLAC)
/// * Episodes: Inferred from URL extension
/// * Livestreams: Determined from stream metadata
struct StreamUrl {
    /// HTTP stream for downloading content.
    stream: HttpStream<reqwest::Client>,
    /// Source URL for codec/quality detection.
    url: reqwest::Url,
}

/// Indicates whether a medium is for the primary track or fallback version.
///
/// When requesting media for playback, the response may be for either:
/// * Primary - The originally requested track
/// * Fallback - An alternative version when primary is unavailable
///
/// If a fallback medium is returned, the track's metadata will be
/// swapped with its fallback version before playback.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum MediumType {
    /// Medium for the primary requested track
    Primary(Medium),
    /// Medium for the fallback version when primary is unavailable
    Fallback(Medium),
}

/// Provides direct access to the underlying `Medium` regardless of variant.
///
/// This allows transparent access to `Medium` methods and fields without
/// explicitly matching on `Primary` vs `Fallback`. Useful when the variant
/// distinction isn't relevant for the operation.
///
/// # Example
///
/// ```
/// use pleezer::track::MediumType;
///
/// let medium_type = MediumType::Primary(medium);
///
/// // Access Medium fields directly through deref
/// let format = medium_type.format;
/// let sources = &medium_type.sources;
/// ```
impl Deref for MediumType {
    type Target = Medium;

    #[inline]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Primary(medium) | Self::Fallback(medium) => medium,
        }
    }
}

impl Track {
    /// Duration of audio to prefetch before playback starts.
    ///
    /// A 3 second buffer provides:
    /// * Enough data to start decoding
    /// * Time to download more data
    /// * Protection against minor network issues
    /// * Reasonable startup latency
    pub const PREFETCH_DURATION: Duration = Duration::from_secs(3);

    /// Default prefetch size in bytes when bitrate is unknown.
    ///
    /// Used when:
    /// * Server doesn't provide Content-Length
    /// * Track bitrate cannot be determined
    /// * External content has no bitrate info
    ///
    /// Value of 60KB matches official client behavior.
    const PREFETCH_DEFAULT: usize = 60 * 1024;

    /// Returns the track's unique identifier.
    #[must_use]
    #[inline]
    pub fn id(&self) -> TrackId {
        self.id
    }

    /// Returns the track duration.
    ///
    /// The duration represents the total playback time of the track.
    /// No duration is available for livestreams.
    #[must_use]
    #[inline]
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Returns whether this content is accessible.
    ///
    /// Always true for songs. Episodes and livestreams may be
    /// region-restricted or temporarily unavailable.
    #[must_use]
    #[inline]
    pub fn available(&self) -> bool {
        self.available
    }

    /// Returns the track type.
    #[must_use]
    #[inline]
    pub fn typ(&self) -> TrackType {
        self.typ
    }

    /// Returns the track's replay gain value if available.
    ///
    /// Replay gain is used for volume normalization:
    /// * Positive values indicate track is quieter than reference
    /// * Negative values indicate track is louder than reference
    /// * None indicates no gain information available
    #[must_use]
    #[inline]
    pub fn gain(&self) -> Option<f32> {
        self.gain
    }

    /// Returns the track title.
    #[must_use]
    #[inline]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Returns the track artist name.
    #[must_use]
    #[inline]
    pub fn artist(&self) -> &str {
        &self.artist
    }

    /// Returns the album title for this track.
    #[must_use]
    #[inline]
    pub fn album_title(&self) -> Option<&str> {
        self.album_title.as_deref()
    }

    /// Returns the cover art identifier for this track.
    ///
    /// Returns:
    /// * Album cover ID for songs (use with "<https://cdn-images.dzcdn.net/images/cover>")
    /// * Podcast artwork ID for episodes (use with "<https://cdn-images.dzcdn.net/images/talk>")
    /// * Station logo ID for livestreams (use with "<https://cdn-images.dzcdn.net/images/cover>")
    ///
    /// Append "/{id}/{resolution}x{resolution}.{format}" where:
    /// * `resolution` is the desired size in pixels (up to 1920)
    /// * `format` is either `jpg` (smaller) or `png` (higher quality)
    ///
    /// Deezer's default is 500x500.jpg
    #[must_use]
    #[inline]
    pub fn cover_id(&self) -> &str {
        &self.cover_id
    }

    /// Returns the track's expiration time.
    ///
    /// After this time, the track becomes unavailable for download
    /// and may need token refresh.
    #[must_use]
    #[inline]
    pub fn expiry(&self) -> Option<SystemTime> {
        self.expiry
    }

    /// Returns whether this is a livestream.
    ///
    /// Livestreams have different behaviors:
    /// * No fixed duration
    /// * Progress always reports 100%
    /// * Multiple quality/codec options
    #[must_use]
    #[inline]
    pub fn is_livestream(&self) -> bool {
        self.typ == TrackType::Livestream
    }

    /// Returns the duration of audio data currently buffered.
    ///
    /// This represents how much of the track has been downloaded and
    /// is available for playback. Used to:
    /// * Track download progress
    /// * Limit seeks to available data
    /// * Prevent blocking on unbuffered seeks
    ///
    /// For livestreams, this always returns `None` since they are continuous
    /// streams without a fixed duration or buffer concept.
    ///
    /// # Panics
    ///
    /// Returns last known value if lock is poisoned due to download task panic.
    #[must_use]
    pub fn buffered(&self) -> Option<Duration> {
        // Return the buffered duration, or when the lock is poisoned because
        // the download task panicked, return the last value before the panic.
        // Practically, this should mean that this track will never be fully
        // buffered.
        *self.buffered.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the track's audio quality.
    #[must_use]
    #[inline]
    pub fn quality(&self) -> AudioQuality {
        self.quality
    }

    /// Returns the encryption cipher used for this track.
    #[must_use]
    #[inline]
    pub fn cipher(&self) -> Cipher {
        self.cipher
    }

    /// Returns whether the track is encrypted.
    ///
    /// True if the track uses any cipher other than NONE.
    #[must_use]
    #[inline]
    pub fn is_encrypted(&self) -> bool {
        self.cipher != Cipher::NONE
    }

    /// Returns whether the track is lossless audio.
    ///
    /// True only for FLAC encoded songs. Episodes and livestreams
    /// are never lossless.
    #[must_use]
    #[inline]
    pub fn is_lossless(&self) -> bool {
        self.codec().is_some_and(|codec| codec == Codec::FLAC)
    }

    /// Returns whether the track is a podcast episode.
    ///
    /// Episodes are external content with:
    /// * Variable bitrate encoding
    /// * Mono audio (by default)
    /// * Direct streaming URLs
    #[must_use]
    #[inline]
    pub fn is_podcast(&self) -> bool {
        self.typ == TrackType::Episode
    }

    /// Cipher format for 64kbps MP3 files using Blowfish CBC stripe encryption.
    const BF_CBC_STRIPE_MP3_64: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_64,
    };

    /// Cipher format for 128kbps MP3 files using Blowfish CBC stripe encryption.
    const BF_CBC_STRIPE_MP3_128: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_128,
    };

    /// Cipher format for 320kbps MP3 files using Blowfish CBC stripe encryption.
    const BF_CBC_STRIPE_MP3_320: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_320,
    };

    /// Cipher format for MP3 files with unknown bitrate using Blowfish CBC stripe encryption.
    const BF_CBC_STRIPE_MP3_MISC: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_MISC,
    };

    /// Cipher format for FLAC files using Blowfish CBC stripe encryption.
    const BF_CBC_STRIPE_FLAC: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::FLAC,
    };

    /// Available cipher formats for basic quality.
    const CIPHER_FORMATS_MP3_64: [CipherFormat; 2] =
        [Self::BF_CBC_STRIPE_MP3_64, Self::BF_CBC_STRIPE_MP3_MISC];

    /// Available cipher formats for standard quality.
    const CIPHER_FORMATS_MP3_128: [CipherFormat; 3] = [
        Self::BF_CBC_STRIPE_MP3_128,
        Self::BF_CBC_STRIPE_MP3_64,
        Self::BF_CBC_STRIPE_MP3_MISC,
    ];

    /// Available cipher formats for high quality.
    const CIPHER_FORMATS_MP3_320: [CipherFormat; 4] = [
        Self::BF_CBC_STRIPE_MP3_320,
        Self::BF_CBC_STRIPE_MP3_128,
        Self::BF_CBC_STRIPE_MP3_64,
        Self::BF_CBC_STRIPE_MP3_MISC,
    ];

    /// Available cipher formats for lossless quality.
    const CIPHER_FORMATS_FLAC: [CipherFormat; 5] = [
        Self::BF_CBC_STRIPE_FLAC,
        Self::BF_CBC_STRIPE_MP3_320,
        Self::BF_CBC_STRIPE_MP3_128,
        Self::BF_CBC_STRIPE_MP3_64,
        Self::BF_CBC_STRIPE_MP3_MISC,
    ];

    /// API endpoint for retrieving media sources.
    const MEDIA_ENDPOINT: &'static str = "v1/get_url";

    fn get_external_medium(&self, quality: AudioQuality) -> Result<MediumType> {
        let external_url = self.external_url.as_ref().ok_or_else(|| {
            Error::unavailable(format!("external {} {self} has no urls", self.typ))
        })?;

        let sources = match external_url {
            ExternalUrl::Direct(url) => {
                vec![media::Source {
                    url: url.clone(),
                    provider: String::default(),
                }]
            }
            ExternalUrl::WithQuality(codec_urls) => {
                // Filter out sources that are of higher quality than requested.
                let mut urls = Vec::new();
                for (bitrate, codec_url) in codec_urls.sort_by_bitrate().into_iter().rev() {
                    if quality.bitrate().is_none_or(|kbps| bitrate <= kbps) {
                        // Prefer AAC over MP3 if both are available for the same bitrate.
                        if let Some(url) = codec_url.aac.or(codec_url.mp3) {
                            urls.push(media::Source {
                                url,
                                provider: String::default(),
                            });
                        }
                    }
                }
                urls
            }
        };

        if sources.is_empty() {
            return Err(Error::unavailable(format!(
                "no valid sources found for external {} {self}",
                self.typ
            )));
        }

        let medium = Medium {
            format: Format::EXTERNAL,
            cipher: media::CipherType { typ: Cipher::NONE },
            sources,
            not_before: None,
            expiry: None,
            media_type: media::Type::FULL,
        };

        Ok(MediumType::Primary(medium))
    }

    /// Retrieves a media source for the track.
    ///
    /// Attempts to get download URLs for the requested quality level,
    /// falling back to lower qualities if necessary.
    ///
    /// # Arguments
    ///
    /// * `client` - HTTP client for API requests
    /// * `media_url` - Base URL for media content
    /// * `quality` - Preferred audio quality
    /// * `license_token` - Token authorizing media access
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Track has expired
    /// * Quality level is unknown
    /// * Media source unavailable
    /// * Network request fails
    /// * HTTP response status is not successful (not 2xx)
    /// * Response parsing fails
    ///
    /// # Quality Fallback
    ///
    /// If requested quality unavailable, attempts lower qualities in order:
    /// * FLAC → MP3 320 → MP3 128 → MP3 64
    /// * MP3 320 → MP3 128 → MP3 64
    /// * MP3 128 → MP3 64
    ///
    /// # Track Fallback
    ///
    /// If no media is available for the primary track, but a fallback track
    /// exists and has available media, returns `MediumType::Fallback`. The
    /// track's metadata will be swapped with the fallback version when
    /// playback begins.
    pub async fn get_medium(
        &self,
        client: &http::Client,
        media_url: &Url,
        quality: AudioQuality,
        license_token: impl Into<String>,
    ) -> Result<MediumType> {
        if !self.available() {
            return Err(Error::unavailable(format!(
                "{} {self} is not available for download",
                self.typ
            )));
        }

        if let Some(expiry) = self.expiry
            && expiry <= SystemTime::now() {
                return Err(Error::unavailable(format!(
                    "{} {self} has expired since {}",
                    self.typ,
                    OffsetDateTime::from(expiry)
                )));
            }

        if self.external {
            return self.get_external_medium(quality);
        }

        let track_token = self.token.as_ref().ok_or_else(|| {
            Error::permission_denied(format!("{} {self} does not have a track token", self.typ))
        })?;

        let mut track_tokens = vec![track_token.to_owned()];
        if let Some(fallback) = &self.fallback
            && let Some(fallback_token) = fallback.token.as_ref() {
                track_tokens.push(fallback_token.to_owned());
            }

        let cipher_formats = match quality {
            AudioQuality::Basic => Self::CIPHER_FORMATS_MP3_64.to_vec(),
            AudioQuality::Standard => Self::CIPHER_FORMATS_MP3_128.to_vec(),
            AudioQuality::High => Self::CIPHER_FORMATS_MP3_320.to_vec(),
            AudioQuality::Lossless => Self::CIPHER_FORMATS_FLAC.to_vec(),
            AudioQuality::Unknown => {
                return Err(Error::unknown(format!(
                    "unknown audio quality for {} {self}",
                    self.typ
                )));
            }
        };

        let request = media::Request {
            license_token: license_token.into(),
            track_tokens,
            media: vec![media::Media {
                typ: media::Type::FULL,
                cipher_formats,
            }],
        };

        // Do not use `client.unlimited` but instead apply rate limiting.
        // This is to prevent hammering the Deezer API in case of deserialize errors.
        let get_url = media_url.join(Self::MEDIA_ENDPOINT)?;
        let body = serde_json::to_string(&request)?;

        let request = client.json(get_url, body);
        let response = client.execute(request).await?;
        let body = response.text().await?;
        let items: media::Response = protocol::json(&body, Self::MEDIA_ENDPOINT)?;

        // Find the first media source that is available.
        // There are as many media objects as there are track tokens.
        let mut result = None;
        for i in 0..items.data.len() {
            if let Data::Media { media } = &items.data[i]
                && let Some(medium) = media.first().cloned() {
                    let medium_type = if i == 0 {
                        MediumType::Primary(medium)
                    } else {
                        MediumType::Fallback(medium)
                    };
                    result = Some(medium_type);
                    break;
                }
        }

        let result = result
            .ok_or_else(|| Error::not_found(format!("no media data for {} {self}", self.typ)))?;

        let available_quality = AudioQuality::from(result.format);

        // User-uploaded tracks are not reported with any quality. We could estimate the quality
        // based on the bitrate, but the official client does not do this either.
        if !self.is_user_uploaded() && !self.is_external() && quality != available_quality {
            warn!(
                "requested {} {self} in {}, but got {}",
                self.typ, quality, available_quality
            );
        }

        Ok(result)
    }

    /// Returns whether this is a user-uploaded track.
    ///
    /// User uploads are identified by negative IDs and only
    /// available for songs.
    #[must_use]
    #[inline]
    pub fn is_user_uploaded(&self) -> bool {
        self.id.is_negative()
    }

    #[must_use]
    #[inline]
    pub fn is_deezer(&self) -> bool {
        self.typ == TrackType::Song && !self.is_user_uploaded()
    }

    #[must_use]
    #[inline]
    pub fn is_cbr(&self) -> bool {
        self.is_deezer() && !self.is_lossless()
    }

    /// Opens a stream for downloading or streaming content.
    ///
    /// Behavior varies by content type:
    /// * Songs - Downloads encrypted content
    /// * Episodes - Opens direct stream
    /// * Livestreams - Opens selected quality stream
    ///
    /// # Arguments
    ///
    /// * `client` - HTTP client for requests
    /// * `medium` - Media source information
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No valid source found
    /// * Track unavailable
    /// * Network error occurs
    /// * HTTP response status is not successful (not 2xx)
    /// * Download cannot start
    async fn open_stream(&self, client: &http::Client, medium: &Medium) -> Result<StreamUrl> {
        let now = SystemTime::now();

        // Deezer usually returns multiple sources for a track. The official
        // client seems to always use the first one. We start with the first
        // and continue with the next one if the first one fails to start.
        for source in &medium.sources {
            // URLs can theoretically be non-HTTP, and we only support HTTP(S) URLs.
            let Some(host_str) = source.url.host_str() else {
                warn!("skipping source with invalid host for {} {self}", self.typ);
                continue;
            };

            // Check if the track is in a timeframe where it can be downloaded.
            // If not, it can be that the download link expired and needs to be
            // refreshed, that the track is not available yet, or that the track is
            // no longer available.
            if let Some(not_before) = medium.not_before
                && not_before > now {
                    warn!(
                        "{} {self} is not available for download until {} from {host_str}",
                        self.typ,
                        OffsetDateTime::from(not_before)
                    );
                    continue;
                }
            if let Some(expiry) = medium.expiry
                && expiry <= now {
                    warn!(
                        "{} {self} is no longer available for download since {} from {host_str}",
                        self.typ,
                        OffsetDateTime::from(expiry)
                    );
                    continue;
                }

            // Perform the request and stream the response.
            match HttpStream::new(client.unlimited.clone(), source.url.clone()).await {
                Ok(stream) => {
                    debug!("starting download of {} {self} from {host_str}", self.typ);
                    return Ok(StreamUrl {
                        stream,
                        url: source.url.clone(),
                    });
                }
                Err(err) => {
                    warn!(
                        "failed to start download of {} {self} from {host_str}: {err}",
                        self.typ
                    );
                }
            }
        }

        Err(Error::unavailable(format!(
            "no valid sources found for {} {self}",
            self.typ
        )))
    }

    fn init_download(&mut self, url: &Url) {
        // Determine the codec and bitrate of the track.
        if let Some(ExternalUrl::WithQuality(urls)) = &self.external_url {
            // Livestreams specify the codec and bitrate with the URL.
            let result = find_codec_bitrate(urls, url);
            self.codec = result.map(|some| some.0);
            self.bitrate = result.map(|some| some.1);
        } else {
            // For episodes, we can infer the codec from the URL.
            if let Some(ExternalUrl::Direct(url)) = &self.external_url {
                if let Some(extension) = url.path().split('.').next_back()
                    && let Ok(codec) = extension.parse() {
                        self.codec = Some(codec);
                    }
            } else if self.is_user_uploaded() {
                self.codec = Some(Codec::MP3);
            } else {
                self.codec = self.quality.codec();
            }

            // For songs, the audio quality determines the codec. When the codec
            // is MP3, the bitrate is constant and determined by the quality. For
            // FLAC, the bitrate is variable and determined by the file size and
            // duration.
            //
            // For episodes, we have no metadata and must rely on the file size
            // and duration to determine the bitrate. This is not perfect, but it
            // is a good approximation.
            self.bitrate = match self.quality {
                AudioQuality::Lossless | AudioQuality::Unknown => {
                    self.file_size
                        .unwrap_or_default()
                        .checked_div(self.duration.unwrap_or_default().as_secs())
                        .map(|bytes| {
                            let mut kbps = usize::try_from(bytes * 8 / 1000).unwrap_or(usize::MAX);

                            // Limit the bitrate to the maximum allowed by the quality.
                            // This is to prevent the bitrate from being too high due to
                            // metadata and visuals in the file.
                            let max_bitrate = match self.codec() {
                                Some(Codec::ADTS | Codec::MP4) => 576,
                                Some(Codec::MP3) => 320,
                                Some(Codec::FLAC) => 1411,
                                Some(Codec::WAV) => 3072,
                                None => usize::MAX,
                            };
                            kbps = kbps.min(max_bitrate);
                            kbps
                        })
                }
                _ => self.quality.bitrate(),
            };
        }
    }

    /// Starts downloading the track.
    ///
    /// Initiates background download and creates `AudioFile` that:
    /// * Streams content from source
    /// * Handles format detection
    /// * Manages encryption if needed
    /// * Tracks download progress
    /// * Enables playback before completion
    ///
    /// # Arguments
    ///
    /// * `client` - HTTP client for download
    /// * `medium` - Media source information
    /// * `storage` - Storage provider with prefetch buffer
    ///
    /// # Returns
    ///
    /// Returns an `AudioFile` instance that provides:
    /// * Unified Read/Seek interface
    /// * Transparent encryption handling
    /// * Format-specific optimizations
    /// * Buffer management
    ///
    /// # Fallback Handling
    ///
    /// If a fallback medium is provided, the track's metadata will be swapped
    /// with its fallback version before download begins. This ensures the
    /// playing track matches the actual content being downloaded.
    ///
    /// The original track metadata is preserved in the fallback field and can
    /// be restored if needed.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No valid source found
    /// * Track unavailable
    /// * Network error occurs
    /// * Download cannot start
    ///
    /// # Progress Tracking
    ///
    /// Download progress is tracked via:
    /// * `buffered()` - Amount downloaded
    /// * `is_complete()` - Download status
    /// * `file_size()` - Total size if known
    ///
    /// # Panics
    ///
    /// * When the buffered duration mutex is poisoned in the progress callback
    /// * When duration calculation overflows during progress calculation
    pub async fn start_download<P>(
        &mut self,
        client: &http::Client,
        medium: &MediumType,
        storage: P,
    ) -> Result<AudioFile>
    where
        P: StorageProvider + Sync + 'static,
        P::Reader: Sync,
    {
        let medium = match medium {
            MediumType::Primary(medium) => medium,
            MediumType::Fallback(medium) => {
                if let Some(fallback) = &mut self.fallback {
                    warn!("falling back {} {} to {fallback}", self.typ, self.id);
                    std::mem::swap(&mut self.id, &mut fallback.id);
                    std::mem::swap(&mut self.artist, &mut fallback.artist);
                    std::mem::swap(&mut self.album_title, &mut fallback.album_title);
                    std::mem::swap(&mut self.cover_id, &mut fallback.cover_id);
                    std::mem::swap(&mut self.duration, &mut fallback.duration);
                    std::mem::swap(&mut self.title, &mut fallback.title);
                    std::mem::swap(&mut self.gain, &mut fallback.gain);
                    std::mem::swap(&mut self.token, &mut fallback.token);
                    std::mem::swap(&mut self.expiry, &mut fallback.expiry);
                }
                medium
            }
        };

        let stream_url = self.open_stream(client, medium).await?;
        let stream = stream_url.stream;
        let url = stream_url.url;

        // Set actual audio quality and cipher type.
        self.quality = medium.format.into();
        self.cipher = medium.cipher.typ;

        // Set the file size if known. This is used to calculate the prefetch size.
        if let Some(file_size) = stream.content_length() {
            if file_size > 0 {
                info!("downloading {file_size} bytes for {} {self}", self.typ);
                self.file_size = Some(file_size);
            } else {
                return Err(Error::data_loss(format!("{} is 0 bytes", self.typ)));
            }
        } else {
            info!("downloading {} {self} with unknown file size", self.typ);
        }

        self.init_download(&url);

        // Calculate the prefetch size based on the bitrate and duration.
        let prefetch_size = self.prefetch_size().try_into()?;
        trace!(
            "prefetch size for {} {self}: {prefetch_size} bytes",
            self.typ
        );

        // A progress callback that logs the download progress.
        let track_str = self.to_string();
        let track_typ = self.typ.to_string();
        let duration = self.duration;
        let buffered = Arc::clone(&self.buffered);
        let file_size = self.file_size;
        let callback = move |_: &HttpStream<_>,
                             stream: StreamState,
                             _: &tokio_util::sync::CancellationToken| {
            match stream.phase {
                StreamPhase::Complete => {
                    info!("completed download of {track_typ} {track_str}");

                    // Prevent rounding errors and set the buffered duration
                    // equal to the total duration. It's OK to unwrap here: if
                    // the mutex is poisoned, then the main thread panicked and
                    // we should propagate the error.
                    *buffered.lock().unwrap() = duration;
                }
                StreamPhase::Downloading { .. } => {
                    if let Some(file_size) = file_size
                        && file_size > 0 {
                            // `f64` not for precision, but to be able to fit
                            // as big as possible file sizes.
                            // TODO : use `Percentage` type
                            #[expect(clippy::cast_precision_loss)]
                            let progress = stream.current_position as f64 / file_size as f64;

                            // OK to unwrap: see rationale above.
                            *buffered.lock().unwrap() = duration.map(|duration| {
                                duration
                                    .mul_f64(progress)
                                    // Subtract the prefetch duration to prevent seeks to a position
                                    // just before the end of the buffered data. When the read block
                                    // extends beyond the buffered data, the download would block to
                                    // prefetch what is beyond the buffered data.
                                    .saturating_sub(Self::PREFETCH_DURATION)
                            });
                        }
                }
                _ => {
                    // Read requests are not allowed during prefetching, so don't
                    // update the buffered duration here: we couldn't read it anyway.
                }
            }
        };

        // Start the download. The `await` here will *not* block until the download is complete,
        // but only until the download is started. The download will continue in the background.
        let download = StreamDownload::from_stream(
            stream,
            storage,
            stream_download::Settings::default()
                .on_progress(callback)
                .prefetch_bytes(prefetch_size)
                .cancel_on_drop(true),
        )
        .await?;

        self.handle = Some(download.handle());
        AudioFile::try_from_download(self, download)
    }

    /// Returns the current download handle if active.
    ///
    /// Returns None if:
    /// * Download hasn't started
    /// * Download was reset
    #[must_use]
    #[inline]
    pub fn handle(&self) -> Option<StreamHandle> {
        self.handle.clone()
    }

    /// Returns whether the track download is complete.
    ///
    /// For livestreams, always returns false since they are continuous
    /// streams that can't be fully buffered.
    #[must_use]
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.buffered() >= self.duration
    }

    /// Resets the track's download state.
    ///
    /// Clears:
    /// * Download handle
    /// * File size information
    /// * Buffer progress
    ///
    /// For livestreams, this will clear any accumulated playback duration
    /// since they don't have a traditional buffer concept.
    ///
    /// Useful when needing to restart an interrupted download or stream.
    ///
    /// # Panics
    ///
    /// Panics if the buffered lock is poisoned.
    pub fn reset_download(&mut self) {
        self.handle = None;
        self.file_size = None;
        *self.buffered.lock().unwrap() = None;
    }

    /// Returns the total file size if known.
    ///
    /// Size becomes available after download starts and server
    /// provides Content-Length.
    #[must_use]
    #[inline]
    pub fn file_size(&self) -> Option<u64> {
        self.file_size
    }

    /// Returns whether this track uses external streaming.
    ///
    /// External tracks:
    /// * Use direct streaming URLs instead of Deezer's CDN
    /// * Are never encrypted
    /// * Include episodes and livestreams
    #[must_use]
    #[inline]
    pub fn is_external(&self) -> bool {
        self.external
    }

    /// Returns the audio bitrate in kbps if known.
    ///
    /// The bitrate may be:
    /// * Fixed (MP3)
    /// * Variable (FLAC)
    /// * Stream-specific (livestreams)
    /// * Unknown (some external content)
    #[must_use]
    #[inline]
    pub fn bitrate(&self) -> Option<usize> {
        self.bitrate
    }

    /// Returns the audio codec used for this content.
    ///
    /// Possible codecs:
    /// * ADTS - Some livestreams
    /// * FLAC - High quality songs only
    /// * MP3 - Most common, used for all content types
    /// * MP4 - Some episodes
    /// * WAV - Some episodes
    #[must_use]
    #[inline]
    pub fn codec(&self) -> Option<Codec> {
        self.codec
    }

    /// Returns the size of audio data to prefetch before playback.
    ///
    /// The prefetch size is calculated based on:
    /// * Track bitrate (if known)
    /// * Prefetch duration constant (3 seconds)
    /// * Default size fallback (60KB)
    ///
    /// # Calculation
    ///
    /// For known bitrates:
    /// ```text
    /// prefetch_size = (bitrate_kbps * 1000 / 8) * prefetch_duration_secs
    /// ```
    ///
    /// For unknown bitrates:
    /// * Uses default size of 60KB
    ///
    /// # Examples
    ///
    /// * 320kbps MP3: ~120KB prefetch
    /// * 128kbps MP3: ~48KB prefetch
    /// * Unknown bitrate: 60KB prefetch
    ///
    /// This size is used for:
    /// * Initial buffering before playback starts
    /// * Configuring storage buffer size
    #[must_use]
    pub fn prefetch_size(&self) -> usize {
        let mut prefetch_size = Self::PREFETCH_DEFAULT;
        if let Some(kbps) = self.bitrate {
            prefetch_size = (kbps * 1000 / 8)
                * Self::PREFETCH_DURATION
                    .as_secs()
                    .try_into()
                    .unwrap_or(usize::MAX);
        }
        prefetch_size
    }
}

/// Creates a Track from gateway list data.
///
/// Initializes track with:
/// * Content type-specific fields
/// * Default quality (Standard)
/// * Default cipher (`BF_CBC_STRIPE`)
/// * Empty download state
///
/// Content types are handled differently:
/// * Songs - Uses artist/album metadata
/// * Episodes - Uses show/podcast metadata and external URLs
/// * Livestreams - Uses station metadata and quality streams
impl From<gateway::ListData> for Track {
    fn from(item: gateway::ListData) -> Self {
        let (gain, album_title) = if let gateway::ListData::Song {
            gain, album_title, ..
        } = &item
        {
            (gain.as_ref(), Some(album_title))
        } else {
            (None, None)
        };

        let (available, external, external_url, fallback) = match &item {
            gateway::ListData::Song { fallback, .. } => (true, false, None, fallback.clone()),
            gateway::ListData::Episode {
                available,
                external,
                external_url,
                ..
            } => (
                *available,
                *external,
                external_url.clone().map(ExternalUrl::Direct),
                None,
            ),
            gateway::ListData::Livestream {
                available,
                external_urls,
                ..
            } => (
                *available,
                true,
                Some(ExternalUrl::WithQuality(external_urls.clone())),
                None,
            ),
        };

        let typ = item.typ().parse().unwrap_or_default();

        Self {
            typ,
            id: item.id(),
            token: item.token().map(ToOwned::to_owned),
            title: item.title().map(ToOwned::to_owned),
            artist: item.artist().to_owned(),
            album_title: album_title.map(ToString::to_string),
            cover_id: item.cover_id().to_owned(),
            duration: item.duration(),
            gain: gain.map(|gain| gain.to_f32_lossy()),
            expiry: item.expiry(),
            quality: AudioQuality::Unknown,
            buffered: Arc::new(Mutex::new(None)),
            file_size: None,
            cipher: Cipher::BF_CBC_STRIPE,
            handle: None,
            available,
            external,
            external_url,
            bitrate: None,
            codec: None,
            sample_rate: None,
            bits_per_sample: None,
            channels: None,
            fallback: fallback.map(|boxed| Box::new((*boxed).into())),
        }
    }
}

/// Formats track for display, showing ID, artist and title if available.
///
/// Format varies by content type:
/// * Songs/Episodes: "{id}: "{artist} - {title}""
/// * Livestreams: "{id}: "{station}""
///
/// # Example
///
/// ```text
/// 12345: "Artist Name - Track Title"
/// ```
impl fmt::Display for Track {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let artist = self.artist();
        if let Some(title) = &self.title() {
            write!(f, "{}: \"{} - {}\"", self.id, artist, title)
        } else {
            write!(f, "{}: \"{}\"", self.id, artist)
        }
    }
}

/// Finds audio format and bitrate for a given stream URL in livestream URLs.
///
/// # Arguments
///
/// * `haystack` - Mapping of bitrates to codec URLs
/// * `needle` - URL to match against codec URLs
///
/// # Returns
///
/// Some((Format, usize)) if the URL is found:
/// * Format - ADTS (for AAC audio) or MP3
/// * usize - Bitrate in kbps
///
/// None if URL is not found in any format/bitrate combination
fn find_codec_bitrate(haystack: &LivestreamUrls, needle: &Url) -> Option<(Codec, usize)> {
    for (kbps, codec) in &haystack.data {
        if codec.aac.as_ref().is_some_and(|aac| aac == needle) {
            return Some((Codec::ADTS, kbps.parse().ok()?));
        } else if codec.mp3.as_ref().is_some_and(|mp3| mp3 == needle) {
            return Some((Codec::MP3, kbps.parse().ok()?));
        }
    }

    None
}
