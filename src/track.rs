use std::{
    fmt,
    num::NonZeroI64,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime},
};

use stream_download::{
    self, http::HttpStream, source::SourceStream, storage::temp::TempStorageProvider,
    StreamDownload, StreamHandle, StreamPhase, StreamState,
};
use time::OffsetDateTime;
use url::Url;

use crate::{
    error::{Error, Result},
    http,
    protocol::{
        connect::AudioQuality,
        gateway,
        media::{self, Cipher, CipherFormat, Data, Format, Medium},
    },
    util::ToF32,
};

/// A unique identifier for a track. User-uploaded tracks are identified by negative IDs.
#[expect(clippy::module_name_repetitions)]
pub type TrackId = NonZeroI64;

#[derive(Debug)]
pub struct Track {
    id: TrackId,
    track_token: String,
    title: String,
    artist: String,
    album_title: String,
    album_cover: String,
    gain: Option<f32>,
    expiry: SystemTime,
    quality: AudioQuality,
    duration: Duration,
    buffered: Arc<Mutex<Duration>>,
    file_size: Option<u64>,
    cipher: Cipher,
    handle: Option<StreamHandle>,
}

impl Track {
    /// Amount of seconds to audio to buffer before the track can be read from.
    const PREFETCH_LENGTH: Duration = Duration::from_secs(3);

    /// The default amount of bytes to prefetch before the track can be read
    /// from. This is used when the track does not provide a `Content-Length`
    /// header, and is equal to what the official Deezer client uses.
    const PREFETCH_DEFAULT: usize = 60 * 1024;

    #[must_use]
    pub fn id(&self) -> TrackId {
        self.id
    }

    #[must_use]
    pub fn duration(&self) -> Duration {
        self.duration
    }

    #[must_use]
    pub fn gain(&self) -> Option<f32> {
        self.gain
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn artist(&self) -> &str {
        &self.artist
    }

    #[must_use]
    pub fn album_title(&self) -> &str {
        &self.album_title
    }

    /// The ID of the album cover image.
    ///
    /// This ID can be used to construct a URL for retrieving the album cover image.
    /// Album covers are always square and available in various resolutions up to 1920x1920.
    ///
    /// # URL Format
    /// ```text
    /// https://e-cdns-images.dzcdn.net/images/cover/{album_cover}/{resolution}x{resolution}.{format}
    /// ```
    /// where:
    /// - `{album_cover}` is the ID returned by this method
    /// - `{resolution}` is the desired resolution in pixels (e.g., 500)
    /// - `{format}` is either `jpg` or `png`
    ///
    /// # Recommended Usage
    /// - Default resolution: 500x500
    /// - Default format: `jpg` (smaller file size)
    /// - Alternative: `png` (higher quality but larger file size)
    ///
    /// # Example
    /// ```text
    /// https://e-cdns-images.dzcdn.net/images/cover/f286f9e7dc818e181c37b944e2461101/500x500.jpg
    /// ```
    #[must_use]
    pub fn album_cover(&self) -> &str {
        &self.album_cover
    }

    #[must_use]
    pub fn expiry(&self) -> SystemTime {
        self.expiry
    }

    /// The duration of the track that has been buffered.
    #[must_use]
    pub fn buffered(&self) -> Duration {
        // Return the buffered duration, or when the lock is poisoned because
        // the download task panicked, return the last value before the panic.
        // Practically, this should mean that this track will never be fully
        // buffered.
        *self.buffered.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn quality(&self) -> AudioQuality {
        self.quality
    }

    #[must_use]
    pub fn cipher(&self) -> Cipher {
        self.cipher
    }

    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.cipher != Cipher::NONE
    }

    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.quality == AudioQuality::Lossless
    }

    const BF_CBC_STRIPE_MP3_64: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_64,
    };

    const BF_CBC_STRIPE_MP3_128: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_128,
    };

    const BF_CBC_STRIPE_MP3_320: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_320,
    };

    const BF_CBC_STRIPE_MP3_MISC: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::MP3_MISC,
    };

    const BF_CBC_STRIPE_FLAC: CipherFormat = CipherFormat {
        cipher: Cipher::BF_CBC_STRIPE,
        format: Format::FLAC,
    };

    const CIPHER_FORMATS_MP3_64: [CipherFormat; 2] =
        [Self::BF_CBC_STRIPE_MP3_64, Self::BF_CBC_STRIPE_MP3_MISC];

    const CIPHER_FORMATS_MP3_128: [CipherFormat; 3] = [
        Self::BF_CBC_STRIPE_MP3_128,
        Self::BF_CBC_STRIPE_MP3_64,
        Self::BF_CBC_STRIPE_MP3_MISC,
    ];

    const CIPHER_FORMATS_MP3_320: [CipherFormat; 4] = [
        Self::BF_CBC_STRIPE_MP3_320,
        Self::BF_CBC_STRIPE_MP3_128,
        Self::BF_CBC_STRIPE_MP3_64,
        Self::BF_CBC_STRIPE_MP3_MISC,
    ];

    const CIPHER_FORMATS_FLAC: [CipherFormat; 5] = [
        Self::BF_CBC_STRIPE_FLAC,
        Self::BF_CBC_STRIPE_MP3_320,
        Self::BF_CBC_STRIPE_MP3_128,
        Self::BF_CBC_STRIPE_MP3_64,
        Self::BF_CBC_STRIPE_MP3_MISC,
    ];

    /// The endpoint for obtaining media sources.
    const MEDIA_ENDPOINT: &'static str = "v1/get_url";

    /// Get a HTTP media source for the track.
    ///
    /// # Parameters
    ///
    /// - `client`: The HTTP client to use for the request.
    /// - `quality`: The audio quality that is preferred.
    /// - `license_token`: The license token to obtain the track with the given quality.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested audio quality is unknown, or if the
    /// media source could not be retrieved.
    ///
    /// # Panics
    ///
    /// Panics if the download state lock is poisoned.
    pub async fn get_medium(
        &self,
        client: &http::Client,
        media_url: &Url,
        quality: AudioQuality,
        license_token: impl Into<String>,
    ) -> Result<Medium> {
        if self.expiry <= SystemTime::now() {
            return Err(Error::unavailable(format!(
                "track {self} no longer available since {}",
                OffsetDateTime::from(self.expiry)
            )));
        }

        let cipher_formats = match quality {
            AudioQuality::Basic => Self::CIPHER_FORMATS_MP3_64.to_vec(),
            AudioQuality::Standard => Self::CIPHER_FORMATS_MP3_128.to_vec(),
            AudioQuality::High => Self::CIPHER_FORMATS_MP3_320.to_vec(),
            AudioQuality::Lossless => Self::CIPHER_FORMATS_FLAC.to_vec(),
            AudioQuality::Unknown => {
                return Err(Error::unknown("unknown audio quality for track {self}"));
            }
        };

        let request = media::Request {
            license_token: license_token.into(),
            track_tokens: vec![self.track_token.clone()],
            media: vec![media::Media {
                typ: media::Type::FULL,
                cipher_formats,
            }],
        };

        // Do not use `client.unlimited` but instead apply rate limiting.
        // This is to prevent hammering the Deezer API in case of deserialize errors.
        let get_url = media_url.join(Self::MEDIA_ENDPOINT)?;
        let body = serde_json::to_string(&request)?;
        let request = client.post(get_url, body);
        let response = client.execute(request).await?;
        let result = response.json::<media::Response>().await?;

        // Deezer only sends a single media object.
        let result = match result.data.first() {
            Some(data) => match data {
                Data::Media { media } => media.first().cloned().ok_or(Error::not_found(
                    format!("empty media data for track {self}"),
                ))?,
                Data::Errors { errors } => {
                    return Err(Error::unavailable(errors.first().map_or_else(
                        || format!("unknown error getting media for track {self}"),
                        ToString::to_string,
                    )));
                }
            },
            None => return Err(Error::not_found(format!("no media data for track {self}"))),
        };

        trace!("get_url: {result:#?}");

        let available_quality = AudioQuality::from(result.format);

        // User-uploaded tracks are not reported with any quality. We could estimate the quality
        // based on the bitrate, but the official client does not do this either.
        if !self.is_user_uploaded() && quality != available_quality {
            warn!(
                "requested track {self} in {}, but got {}",
                quality, available_quality
            );
        }

        Ok(result)
    }

    #[must_use]
    pub fn is_user_uploaded(&self) -> bool {
        self.id.is_negative()
    }

    async fn open_stream(
        &self,
        client: &http::Client,
        medium: &Medium,
    ) -> Result<HttpStream<reqwest::Client>> {
        let mut result = Err(Error::unavailable(format!(
            "no valid sources found for track {self}"
        )));

        let now = SystemTime::now();

        // Deezer usually returns multiple sources for a track. The official
        // client seems to always use the first one. We start with the first
        // and continue with the next one if the first one fails to start.
        #[expect(clippy::iter_next_slice)]
        while let Some(source) = medium.sources.iter().next() {
            // URLs can theoretically be non-HTTP, and we only support HTTP(S) URLs.
            let Some(host_str) = source.url.host_str() else {
                warn!("skipping source with invalid host for track {self}");
                continue;
            };

            // Check if the track is in a timeframe where it can be downloaded.
            // If not, it can be that the download link expired and needs to be
            // refreshed, that the track is not available yet, or that the track is
            // no longer available.
            if medium.not_before > now {
                warn!(
                    "track {self} is not available for download until {} from {host_str}",
                    OffsetDateTime::from(medium.not_before)
                );
                continue;
            }
            if medium.expiry <= now {
                warn!(
                    "track {self} is no longer available for download since {} from {host_str}",
                    OffsetDateTime::from(medium.expiry)
                );
                continue;
            }

            // Perform the request and stream the response.
            match HttpStream::new(client.unlimited.clone(), source.url.clone()).await {
                Ok(http_stream) => {
                    debug!("starting download of track {self} from {host_str}");
                    result = Ok(http_stream);
                    break;
                }
                Err(err) => {
                    warn!("failed to start download of track {self} from {host_str}: {err}",);
                    continue;
                }
            };
        }

        result
    }

    /// Start downloading the track with the given `client` and from the given
    /// `medium`. The download will be started in the background.
    ///
    /// # Errors
    ///
    /// Returns an error if the no sources are found for the track, if the URL
    /// has no host name, if the track is not available for download, or if the
    /// download link expired.
    ///
    /// # Panics
    ///
    /// Panics if the download state lock is poisoned.
    pub async fn start_download(
        &mut self,
        client: &http::Client,
        medium: &Medium,
    ) -> Result<StreamDownload<TempStorageProvider>> {
        let stream = self.open_stream(client, medium).await?;

        // Set actual audio quality and cipher type.
        self.quality = medium.format.into();
        self.cipher = medium.cipher.typ;

        // Calculate the prefetch size based on the audio quality. This assumes
        // that the track is encoded with a constant bitrate, which is not
        // necessarily true. However, it is a good approximation.
        let mut prefetch_size = None;
        if let Some(file_size) = stream.content_length() {
            info!("downloading {file_size} bytes for track {self}");
            self.file_size = Some(file_size);

            if !self.duration.is_zero() {
                let size = Self::PREFETCH_LENGTH.as_secs()
                    * file_size.saturating_div(self.duration.as_secs());
                trace!("prefetch size for track {self}: {size} bytes");
                prefetch_size = Some(size);
            }
        } else {
            info!("downloading track {self} with unknown file size");
        };
        let prefetch_size = prefetch_size.unwrap_or(Self::PREFETCH_DEFAULT as u64);

        // A progress callback that logs the download progress.
        let track_str = self.to_string();
        let duration = self.duration;
        let buffered = Arc::clone(&self.buffered);
        let callback = move |stream: &HttpStream<_>,
                             stream_state: StreamState,
                             _: &tokio_util::sync::CancellationToken| {
            if stream_state.phase == StreamPhase::Complete {
                info!("completed download of track {track_str}");

                // Prevent rounding errors and set the buffered duration
                // equal to the total duration. It's OK to unwrap here: if
                // the mutex is poisoned, then the main thread panicked and
                // we should propagate the error.
                *buffered.lock().unwrap() = duration;
            } else if let Some(file_size) = stream.content_length() {
                if file_size > 0 {
                    // `f64` not for precision, but to be able to fit
                    // as big as possible file sizes.
                    // TODO : use `Percentage` type
                    #[expect(clippy::cast_precision_loss)]
                    let progress = stream_state.current_position as f64 / file_size as f64;

                    // OK to unwrap: see rationale above.
                    *buffered.lock().unwrap() = duration.mul_f64(progress);
                }
            }
        };

        // Start the download. The `await` here will *not* block until the download is complete,
        // but only until the download is started. The download will continue in the background.
        let download = StreamDownload::from_stream(
            stream,
            TempStorageProvider::default(),
            stream_download::Settings::default()
                .on_progress(callback)
                .prefetch_bytes(prefetch_size),
        )
        .await?;

        self.handle = Some(download.handle());
        Ok(download)
    }

    /// Returns a handle to interact with the download of the track, if the
    /// download has been started.
    #[must_use]
    pub fn handle(&self) -> Option<StreamHandle> {
        self.handle.clone()
    }

    /// Whether the download of the track is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.buffered().as_secs() == self.duration.as_secs()
    }

    /// Reset the download progress and file size of the track. This can be
    /// useful if the download was interrupted and needs to be restarted.
    ///
    /// # Panics
    ///
    /// Panics if the buffered lock is poisoned.
    pub fn reset_download(&mut self) {
        self.handle = None;
        self.file_size = None;
        *self.buffered.lock().unwrap() = Duration::ZERO;
    }

    /// Returns the file size of the track, if known after the download has
    /// started.
    #[must_use]
    pub fn file_size(&self) -> Option<u64> {
        self.file_size
    }
}

impl From<gateway::ListData> for Track {
    fn from(item: gateway::ListData) -> Self {
        Self {
            id: item.track_id,
            track_token: item.track_token,
            title: item.title.to_string(),
            artist: item.artist.to_string(),
            album_title: item.album_title.to_string(),
            album_cover: item.album_cover,
            duration: item.duration,
            gain: item.gain.map(ToF32::to_f32_lossy),
            expiry: item.expiry,
            quality: AudioQuality::Standard,
            buffered: Arc::new(Mutex::new(Duration::ZERO)),
            file_size: None,
            cipher: Cipher::BF_CBC_STRIPE,
            handle: None,
        }
    }
}

impl fmt::Display for Track {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: \"{} - {}\"", self.id, self.artist, self.title)
    }
}
