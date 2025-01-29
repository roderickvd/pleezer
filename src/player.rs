//! Audio playback and track management.
//!
//! This module handles:
//! * Audio device configuration and output
//! * Track playback and decoding (using Symphonia)
//! * Queue management and track access
//! * Volume normalization and control
//!   - Primary: Uses Deezer-provided gain values
//!   - Fallback: `ReplayGain` metadata from external files (e.g., podcasts)
//!   - Target: -15 LUFS with headroom protection
//!   - Dynamic range compression for loud content
//! * Event notifications
//!
//! # Audio Pipeline
//!
//! The playback pipeline consists of:
//! 1. Track download and format handling through `AudioFile` abstraction
//! 2. Format-specific decoding:
//!    * MP3: Fast seeking for CBR streams
//!    * FLAC: Raw frame handling
//!    * AAC: ADTS stream parsing
//!    * WAV: PCM decoding
//! 3. Volume normalization (optional)
//! 4. Logarithmic volume control
//! 5. Fade-out processing for smooth transitions
//! 6. Audio device output
//!
//! # Features
//!
//! * Unified audio stream handling
//! * Optimized CBR MP3 seeking
//! * Track preloading for gapless playback
//! * Volume normalization with limiter
//! * Flexible audio device selection
//! * Multiple audio host support
//!
//! # Example
//!
//! ```rust
//! use pleezer::player::Player;
//!
//! // Create player with default audio device
//! let mut player = Player::new(&config, "").await?;
//!
//! // Configure playback
//! player.set_normalization(true);
//! player.set_volume(volume);
//!
//! // Open the audio device
//! player.start()?;
//!
//! // Add tracks and start playback
//! player.set_queue(tracks);
//! player.play()?;
//!
//! // When done, close the audio device
//! player.stop();
//! ```

use std::{collections::HashSet, sync::Arc, time::Duration};

use cpal::traits::{DeviceTrait, HostTrait};
use md5::{Digest, Md5};
use rodio::Source;
use stream_download::storage::{adaptive::AdaptiveStorageProvider, temp::TempStorageProvider};
use url::Url;

use crate::{
    config::Config,
    decoder::Decoder,
    decrypt::{self},
    error::{Error, ErrorKind, Result},
    events::Event,
    http, normalize,
    protocol::{
        connect::{
            contents::{AudioQuality, RepeatMode},
            Percentage,
        },
        gateway::{self, MediaUrl},
    },
    track::{Track, TrackId, DEFAULT_SAMPLE_RATE},
    util::{self, ToF32, UNITY_GAIN},
};

/// Audio sample type used by the decoder.
///
/// This is the native format that rodio's decoder produces,
/// used for internal audio processing.
pub type SampleFormat = f32;

/// Audio playback manager.
///
/// Handles:
/// * Audio device management
/// * Format-specific decoding via Symphonia
/// * Queue management and ordering
/// * Playback control
/// * Audio parameters:
///   - Sample rate (defaults to 44.1 kHz)
///   - Bits per sample (codec-dependent)
///   - Channel count (content-specific)
/// * Volume normalization:
///   - Primarily uses Deezer-provided gain values
///   - Falls back to `ReplayGain` metadata for external content
///   - Targets -15 LUFS with headroom protection
///   - Applies dynamic range compression when needed
///
/// Format support:
/// * Songs: MP3 (CBR) and FLAC (no `ReplayGain`, uses Deezer gain)
/// * Podcasts: MP3, AAC (ADTS), MP4, WAV (may contain `ReplayGain`)
/// * Livestreams: AAC (ADTS) and MP3
///
/// Audio device lifecycle:
/// * Device specification is stored during construction
/// * Device is opened automatically on first play
/// * Manual `start()` calls are optional
/// * Device is closed with `stop()`
/// * Device state affects method behavior:
///   - Most playback operations require an open device
///   - Configuration can be changed when device is closed
pub struct Player {
    /// Preferred audio quality setting.
    ///
    /// Actual quality may be lower if track isn't available
    /// in the preferred quality.
    audio_quality: AudioQuality,

    /// License token for media access.
    ///
    /// Required for downloading encrypted tracks.
    license_token: String,

    /// Ordered list of tracks for playback.
    /// Order may be changed by shuffle operations.
    queue: Vec<Track>,

    /// Set of track IDs to skip during playback.
    ///
    /// Tracks are added here when they fail to load
    /// or become unavailable.
    skip_tracks: HashSet<TrackId>,

    /// Current position in the queue.
    ///
    /// May exceed queue length to prepare for
    /// future queue updates.
    position: usize,

    /// Position to seek to after track loads.
    ///
    /// Used when seek is requested before track
    /// is fully loaded.
    deferred_seek: Option<Duration>,

    /// HTTP client for downloading tracks.
    ///
    /// Uses cookie-less client as tracks don't
    /// require authentication.
    client: http::Client,

    /// Current repeat mode setting.
    ///
    /// Controls behavior at queue boundaries.
    repeat_mode: RepeatMode,

    /// Whether volume normalization is enabled.
    normalization: bool,

    /// Target gain for volume normalization in dB.
    ///
    /// Used to calculate normalization ratios.
    gain_target_db: i8,

    /// Raw volume setting as a percentage (0.0 to 1.0).
    ///
    /// This stores the user-set volume before logarithmic scaling is applied.
    /// The actual output volume uses logarithmic scaling for better perceived control.
    volume: Percentage,

    /// Channel for sending playback events.
    ///
    /// Events include:
    /// * Play/Pause
    /// * Track changes
    /// * Connection status
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<Event>>,

    /// Audio device specification string.
    ///
    /// Stored during construction and used to configure the device when `start()` is called.
    /// Format: `[<host>][|<device>][|<sample rate>][|<sample format>]`.
    device: String,

    /// Audio output sink.
    ///
    /// Handles final audio output and volume control.
    /// Only available when device is open (between `start()` and `stop()`).
    sink: Option<rodio::Sink>,

    /// Audio output stream handle.
    ///
    /// Must be kept alive to maintain playback.
    /// Only available when device is open (between `start()` and `stop()`).
    stream: Option<rodio::OutputStream>,

    /// Queue of audio sources.
    ///
    /// Contains decoded and processed audio data ready for playback.
    /// Only available when device is open (between `start()` and `stop()`).
    sources: Option<Arc<rodio::queue::SourcesQueueInput<SampleFormat>>>,

    /// When current track started playing.
    ///
    /// Used to calculate playback progress.
    playing_since: Duration,

    /// Completion signal for current track.
    ///
    /// Receiver is notified when track finishes.
    current_rx: Option<std::sync::mpsc::Receiver<()>>,

    /// Completion signal for preloaded track.
    ///
    /// Receiver is notified when preloaded track
    /// would finish. Used for gapless playback.
    preload_rx: Option<std::sync::mpsc::Receiver<()>>,

    /// Base URL for media content.
    ///
    /// Used to construct track download URLs.
    media_url: Url,
}

impl Player {
    /// Default volume level.
    ///
    /// Constant value of 100% (1.0) used as initial volume setting.
    const DEFAULT_VOLUME: Percentage = Percentage::from_ratio_f32(UNITY_GAIN);

    /// Logarithmic volume scale factor for a dynamic range of 60 dB.
    ///
    /// Equal to 10^(60/20) = 1000.0
    /// Constant used in volume scaling calculations.
    const LOG_VOLUME_SCALE_FACTOR: f32 = 1000.0;

    /// Logarithmic volume growth rate for a dynamic range of 60 dB.
    ///
    /// Equal to ln(1000) ≈ 6.907755279
    /// Constant used in volume scaling calculations.
    const LOG_VOLUME_GROWTH_RATE: f32 = 6.907_755_4;

    /// Duration of the fade to prevent audio popping when clearing the queue
    /// or changing volume.
    ///
    /// A short linear ramp (25ms) is applied to avoid abrupt changes and
    /// sudden audio cutoffs that can cause popping sounds.
    const FADE_DURATION: Duration = Duration::from_millis(25);

    /// Creates a new player instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Player configuration including normalization settings
    /// * `device` - Audio device specification string:
    ///   ```text
    ///   [<host>][|<device>][|<sample rate>][|<sample format>]
    ///   ```
    ///   All parts are optional. Use empty string for system default.
    ///   Device configuration is deferred until `start()` is called.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * HTTP client creation fails
    /// * Decryption key is invalid
    pub async fn new(config: &Config, device: &str) -> Result<Self> {
        let client = http::Client::without_cookies(config)?;

        let bf_secret = if let Some(secret) = config.bf_secret {
            secret
        } else {
            debug!("no bf_secret specified, fetching one from the web player");
            Config::try_key(&client).await?
        };

        if format!("{:x}", Md5::digest(*bf_secret)) == Config::BF_SECRET_MD5 {
            decrypt::set_bf_secret(bf_secret)?;
        } else {
            return Err(Error::permission_denied("the bf_secret is not valid"));
        }

        #[expect(clippy::cast_possible_truncation)]
        let gain_target_db = gateway::user_data::Gain::default().target as i8;

        Ok(Self {
            queue: Vec::new(),
            skip_tracks: HashSet::new(),
            position: 0,
            audio_quality: AudioQuality::default(),
            client,
            license_token: String::new(),
            media_url: MediaUrl::default().into(),
            repeat_mode: RepeatMode::default(),
            normalization: config.normalization,
            gain_target_db,
            volume: Self::DEFAULT_VOLUME,
            event_tx: None,
            playing_since: Duration::ZERO,
            deferred_seek: None,
            current_rx: None,
            preload_rx: None,
            device: device.to_owned(),
            sink: None,
            stream: None,
            sources: None,
        })
    }

    /// Selects and configures an audio output device.
    ///
    /// # Arguments
    ///
    /// * `device` - Device specification string in format:
    ///   ```text
    ///   [<host>][|<device>][|<sample rate>][|<sample format>]
    ///   ```
    ///   All parts are optional. Use empty string for system default.
    ///
    /// # Returns
    ///
    /// Returns the selected device and its configuration.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Host is not found
    /// * Device is not found
    /// * Sample rate is invalid
    /// * Sample format is not supported
    /// * Device cannot be acquired (e.g., in use by another application)
    fn get_device(device: &str) -> Result<(rodio::Device, rodio::SupportedStreamConfig)> {
        // The device string has the following format:
        // "[<host>][|<device>][|<sample rate>][|<sample format>]" (case-insensitive)
        // From left to right, the fields are optional, but each field
        // depends on the preceding fields being specified.
        let mut components = device.split('|');

        // The host is the first field.
        let host = match components.next() {
            Some("") | None => cpal::default_host(),
            Some(name) => {
                let host_ids = cpal::available_hosts();
                host_ids
                    .into_iter()
                    .find_map(|host_id| {
                        let host = cpal::host_from_id(host_id).ok()?;
                        if host.id().name().eq_ignore_ascii_case(name) {
                            Some(host)
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| Error::not_found(format!("audio host {name} not found")))?
            }
        };

        // The device is the second field.
        let device = match components.next() {
            Some("") | None => host.default_output_device().ok_or_else(|| {
                Error::not_found(format!(
                    "default audio output device not found on {}",
                    host.id().name()
                ))
            })?,
            Some(name) => {
                let mut devices = host.output_devices()?;
                devices
                    .find(|device| device.name().is_ok_and(|n| n.eq_ignore_ascii_case(name)))
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "audio output device {name} not found on {}",
                            host.id().name()
                        ))
                    })?
            }
        };

        let config = match components.next() {
            Some("") | None => device.default_output_config().map_err(|e| {
                Error::unavailable(format!("default output configuration unavailable: {e}"))
            })?,
            Some(rate) => {
                let rate = rate
                    .parse()
                    .map_err(|_| Error::invalid_argument(format!("invalid sample rate {rate}")))?;
                let rate = cpal::SampleRate(rate);

                let format = match components.next() {
                    Some("") | None => None,
                    other => other,
                };

                device
                    .supported_output_configs()?
                    .find_map(|config| {
                        if format.is_none_or(|format| {
                            config
                                .sample_format()
                                .to_string()
                                .eq_ignore_ascii_case(format)
                        }) {
                            config.try_with_sample_rate(rate)
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| {
                        Error::unavailable(format!(
                            "audio output device {} does not support sample rate {} with {} sample format",
                            device.name().as_deref().unwrap_or("UNKNOWN"),
                            rate.0,
                            format.unwrap_or("default")
                        ))
                    })?
            }
        };

        info!(
            "audio output device: {} on {}",
            device.name().as_deref().unwrap_or("UNKNOWN"),
            host.id().name()
        );

        #[expect(clippy::cast_precision_loss)]
        let sample_rate = config.sample_rate().0 as f32 / 1000.0;
        info!(
            "audio output configuration: {sample_rate:.1} kHz in {}",
            config.sample_format()
        );
        trace!("audio buffer size: {:#?}", config.buffer_size());

        Ok((device, config))
    }

    /// Opens and configures the audio output device for playback if not already open.
    ///
    /// Called internally when needed (e.g., by `play()`) to initialize the audio device.
    /// The device remains open until `stop` is called or the player is dropped.
    ///
    /// Note: Manual calls to this method are not required as device initialization
    /// is handled automatically.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Audio device specification is invalid
    /// * Device is not available
    /// * Device cannot be opened
    /// * Output stream creation fails
    /// * Sink creation fails
    pub fn start(&mut self) -> Result<()> {
        if self.is_started() {
            return Ok(());
        }

        debug!("opening output device");

        let (device, device_config) = Self::get_device(&self.device)?;
        let (stream, handle) = rodio::OutputStream::try_from_device_config(&device, device_config)?;
        let sink = rodio::Sink::try_new(&handle)?;

        // Set the volume to the last known value.
        let target_volume =
            std::mem::replace(&mut self.volume, Percentage::from_ratio_f32(sink.volume()));
        sink.set_volume(target_volume.as_ratio_f32());

        // The output source will output silence when the queue is empty.
        // That will cause the sink to report as "playing", so we need to pause it.
        let (sources, output) = rodio::queue::queue(true);
        sink.append(output);
        sink.pause();

        self.sink = Some(sink);
        self.sources = Some(sources);
        self.stream = Some(stream);

        Ok(())
    }

    /// Closes the audio output device and stops playback.
    ///
    /// Releases audio device resources and clears any queued audio.
    /// The player can be restarted with `start()`.
    ///
    /// Note: This method is automatically called when the player is dropped,
    /// ensuring proper cleanup of audio device resources.
    pub fn stop(&mut self) {
        // Don't care if the sink is already dropped: we're already "stopped".
        if let Ok(sink) = self.sink_mut() {
            debug!("closing output device");
            sink.stop();
        }

        self.sources = None;
        self.stream = None;
        self.sink = None;
    }

    /// The list of sample rates to enumerate.
    ///
    /// Only includes the two most common sample rates in Hz:
    /// * 44100 - CD audio, most streaming services
    /// * 48000 - Professional digital audio, video production, many sound cards
    const SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

    /// The list of sample formats to enumerate.
    ///
    /// Only includes the three most common sample formats:
    /// * I16 - 16-bit signed integer
    /// * I32 - 32-bit signed integer
    /// * F32 - 32-bit floating point
    const SAMPLE_FORMATS: [cpal::SampleFormat; 3] = [
        cpal::SampleFormat::I16,
        cpal::SampleFormat::I32,
        cpal::SampleFormat::F32,
    ];

    /// Lists available audio output devices.
    ///
    /// Returns a sorted list of device specifications in the format:
    /// ```text
    /// <host>|<device>|<sample rate>|<sample format>
    /// ```
    ///
    /// Only enumerates configurations meeting these criteria:
    /// * Standard sample rates:
    ///   - 44.1 kHz (CD audio, streaming services)
    ///   - 48 kHz (professional audio, video production)
    ///   - I16 (16-bit integer)
    ///   - I32 (32-bit integer)
    ///   - F32 (32-bit float)
    /// * Stereo output (2 channels)
    ///
    /// Default device is marked with "(default)" suffix.
    ///
    /// Note: Other device configurations can still be used by explicitly
    /// specifying them in the device string passed to `new()`.
    ///
    /// # Returns
    ///
    /// A vector of device specification strings, as sorted by the host.
    #[must_use]
    pub fn enumerate_devices() -> Vec<String> {
        let hosts = cpal::available_hosts();
        let mut result = Vec::new();

        // Enumerate all available hosts, devices and configs.
        for host in hosts
            .into_iter()
            .filter_map(|id| cpal::host_from_id(id).ok())
        {
            if let Ok(devices) = host.output_devices() {
                for device in devices {
                    if let Ok(device_name) = device.name() {
                        if let Ok(configs) = device.supported_output_configs() {
                            for config in configs {
                                if config.channels() == 2
                                    && Self::SAMPLE_FORMATS.contains(&config.sample_format())
                                {
                                    for sample_rate in &Self::SAMPLE_RATES {
                                        if let Some(config) = config
                                            .try_with_sample_rate(cpal::SampleRate(*sample_rate))
                                        {
                                            let line = format!(
                                                "{}|{}|{}|{}",
                                                host.id().name(),
                                                device_name,
                                                config.sample_rate().0,
                                                config.sample_format(),
                                            );

                                            result.push(line);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Advances to the next track in the queue.
    ///
    /// Handles:
    /// * Repeat mode logic
    /// * Position updates
    /// * Event notifications
    ///
    /// Behavior depends on repeat mode:
    /// * `None`: Stops at end of queue
    /// * `One`: Stays on current track
    /// * `All`: Loops back to start of queue
    fn go_next(&mut self) {
        let old_position = self.position;
        let repeat_mode = self.repeat_mode();
        if repeat_mode != RepeatMode::One {
            let next = self.position.saturating_add(1);
            if next < self.queue.len() {
                // Move to the next track.
                self.position = next;
            } else {
                // Reached the end of the queue: rewind to the beginning.
                if repeat_mode != RepeatMode::All {
                    self.pause();
                };
                self.position = 0;
            }
        }

        if self.position() != old_position {
            self.notify(Event::TrackChanged);
        }

        // Even if we were already playing, we need to report another playback stream.
        if self.is_playing() {
            self.notify(Event::Play);
        }
    }

    /// The normalization attack time (5ms).
    /// This is the time it takes for the limiter to respond to level increases.
    /// Value matches Spotify's implementation for consistent behavior.
    const NORMALIZE_ATTACK_TIME: Duration = Duration::from_millis(5);

    /// The normalization release time (100ms).
    /// This is the time it takes for the limiter to recover after level decreases.
    /// Value matches Spotify's implementation for consistent behavior.
    const NORMALIZE_RELEASE_TIME: Duration = Duration::from_millis(100);

    /// Threshold level where limiting begins.
    /// Set to -1 dB to provide headroom for inter-sample peaks.
    const NORMALIZE_THRESHOLD_DB: f32 = -1.0;

    /// Width of the soft knee in dB.
    /// A 4 dB width provides smooth transition into limiting.
    const NORMALIZE_KNEE_WIDTH_DB: f32 = 4.0;

    /// Time before network operations timeout.
    const NETWORK_TIMEOUT: Duration = Duration::from_secs(2);

    /// The `ReplayGain` 2.0 reference level in LUFS.
    /// Used when calculating normalization from `ReplayGain` metadata.
    const REPLAY_GAIN_LUFS: i8 = -18;

    /// Loads and prepares a track for playback.
    ///
    /// Downloads and configures audio processing:
    /// 1. Downloads content through unified `AudioFile` interface
    /// 2. Configures format-specific decoder:
    ///    * MP3: Optimized seeking for CBR content
    ///    * FLAC: Raw frame handling
    ///    * AAC: ADTS stream parsing
    ///    * WAV: PCM decoding
    /// 3. Detects audio parameters:
    ///    * Sample rate from codec (defaults to 44.1 kHz)
    ///    * Bits per sample if available
    ///    * Channel count from codec or content type
    /// 4. Applies volume normalization if enabled
    ///
    /// # Arguments
    ///
    /// * `position` - Queue position of track to load
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Audio device is not open (no sources available)
    /// * Track download fails
    /// * Audio decoding fails
    // TODO : consider controlflow
    async fn load_track(
        &mut self,
        position: usize,
    ) -> Result<Option<std::sync::mpsc::Receiver<()>>> {
        let track = self
            .queue
            .get_mut(position)
            .ok_or_else(|| Error::not_found(format!("track at position {position} not found")))?;

        let sources = self
            .sources
            .as_mut()
            .ok_or_else(|| Error::unavailable("audio sources not available"))?;

        if track.handle().is_none() {
            let download = tokio::time::timeout(Self::NETWORK_TIMEOUT, async {
                // Start downloading the track.
                let medium = track
                    .get_medium(
                        &self.client,
                        &self.media_url,
                        self.audio_quality,
                        self.license_token.clone(),
                    )
                    .await?;

                let prefetch_size = usize::try_from(track.prefetch_size()).unwrap_or(usize::MAX);
                let storage = AdaptiveStorageProvider::new(
                    TempStorageProvider::default(),
                    prefetch_size
                        .try_into()
                        .map_err(|e| Error::internal(format!("prefetch size error: {e}")))?,
                );
                track.start_download(&self.client, &medium, storage).await
            })
            .await??;

            // Create a new decoder for the track.
            let mut decoder = Decoder::new(track, download)?;
            track.sample_rate = Some(decoder.sample_rate());
            track.channels = Some(decoder.channels());
            if let Some(bits_per_sample) = decoder.bits_per_sample() {
                track.bits_per_sample = Some(bits_per_sample);
            }

            // Seek to the deferred position if set.
            if let Some(progress) = self.deferred_seek.take() {
                // Set the track position only if `progress` is beyond the track start. We start
                // at the beginning anyway, and this prevents decoder errors.
                if !progress.is_zero() {
                    if let Err(e) = decoder.try_seek(progress) {
                        error!("failed to seek to deferred position: {}", e);
                    }
                }
            }

            // Apply volume normalization if enabled.
            let mut difference = 0.0;
            if self.normalization {
                match track.gain() {
                    Some(gain) => difference = f32::from(self.gain_target_db) - gain,
                    None => {
                        if let Some(replay_gain) = decoder.replay_gain() {
                            debug!("track replay gain: {replay_gain:.1} dB");
                            let track_lufs = f32::from(Self::REPLAY_GAIN_LUFS) - replay_gain;
                            difference = f32::from(self.gain_target_db) - track_lufs;
                        } else {
                            warn!(
                                "{} {track} has no gain information, skipping normalization",
                                track.typ()
                            );
                        }
                    }
                }
            };

            let rx = if difference == 0.0 {
                // No normalization needed, just append the decoder.
                sources.append_with_signal(decoder)
            } else {
                let ratio = util::db_to_ratio(difference);
                debug!(
                    "normalizing {} {track} by {difference:.1} dB ({})",
                    track.typ(),
                    Percentage::from_ratio_f32(ratio)
                );
                let normalized = normalize::normalize(
                    decoder,
                    ratio,
                    Self::NORMALIZE_THRESHOLD_DB,
                    Self::NORMALIZE_KNEE_WIDTH_DB,
                    Self::NORMALIZE_ATTACK_TIME,
                    Self::NORMALIZE_RELEASE_TIME,
                );
                sources.append_with_signal(normalized)
            };

            let sample_rate = track.sample_rate.map_or("unknown".to_string(), |rate| {
                (rate.to_f32_lossy() / 1000.).to_string()
            });
            let codec = track
                .codec()
                .map_or("unknown".to_string(), |codec| codec.to_string());
            let bitrate = track
                .bitrate()
                .map_or("unknown".to_string(), |kbps| kbps.to_string());
            debug!(
                "loaded {} {track}; codec: {codec}; sample rate: {sample_rate} kHz; bitrate: {bitrate} kbps; channels: {}",
                track.typ(),
                track.channels.unwrap_or_else(|| track.typ().default_channels())
            );

            return Ok(Some(rx));
        }

        Ok(None)
    }

    /// Returns the current playback position from the sink.
    ///
    /// Returns `Duration::ZERO` if audio device is not open.
    #[must_use]
    fn get_pos(&self) -> Duration {
        // If the sink is not available, we're not playing anything, so the position is 0.
        self.sink
            .as_ref()
            .map_or(Duration::ZERO, rodio::Sink::get_pos)
    }

    /// Main playback loop.
    ///
    /// Continuously:
    /// * Monitors current track completion
    /// * Manages track preloading
    /// * Handles playback transitions
    /// * Processes track unavailability
    ///
    /// Audio playback requires calling `start()` to open the audio device,
    /// but track loading and queue management will work without it.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Track loading fails critically
    /// * Audio system fails
    pub async fn run(&mut self) -> Result<()> {
        loop {
            match self.current_rx.as_mut() {
                Some(current_rx) => {
                    // Check if the current track has finished playing.
                    if current_rx.try_recv().is_ok() {
                        // Save the point in time when the track finished playing.
                        self.playing_since = self.get_pos();

                        // Move the preloaded track, if any, to the current track.
                        self.current_rx = self.preload_rx.take();
                        self.go_next();
                    }

                    // Preload the next track if all of the following conditions are met:
                    // - the repeat mode is not "Repeat One"
                    // - the current track is done downloading
                    if self.preload_rx.is_none()
                        && self.repeat_mode() != RepeatMode::One
                        && self.track().is_some_and(Track::is_complete)
                    {
                        let next_position = self.position.saturating_add(1);
                        if let Some(next_track) = self.queue.get(next_position) {
                            let next_track_id = next_track.id();
                            let next_track_typ = next_track.typ();
                            if !self.skip_tracks.contains(&next_track_id) {
                                match self.load_track(next_position).await {
                                    Ok(rx) => {
                                        self.preload_rx = rx;
                                    }
                                    Err(e) => {
                                        error!("failed to preload next {next_track_typ}: {e}");
                                        self.mark_unavailable(next_track_id);
                                    }
                                }
                            }
                        }
                    }
                }

                None => {
                    if let Some(track) = self.track() {
                        let track_id = track.id();
                        let track_typ = track.typ();
                        if self.skip_tracks.contains(&track_id) {
                            self.go_next();
                        } else {
                            match self.load_track(self.position).await {
                                Ok(rx) => {
                                    if let Some(rx) = rx {
                                        self.current_rx = Some(rx);
                                        self.notify(Event::TrackChanged);
                                        if self.is_playing() {
                                            self.notify(Event::Play);
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("failed to load {track_typ}: {e}");
                                    self.mark_unavailable(track_id);
                                }
                            }
                        }
                    }
                }
            }

            // Yield to the runtime to allow other tasks to run.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Marks a track as unavailable for playback.
    ///
    /// Tracks marked unavailable will be skipped during playback.
    /// Logs a warning the first time a track is marked unavailable.
    fn mark_unavailable(&mut self, track_id: TrackId) {
        if self.skip_tracks.insert(track_id) {
            warn!("marking track {track_id} as unavailable");
        }
    }

    /// Sends a playback event notification.
    ///
    /// Events are sent through the registered channel if available.
    /// Failures are logged but do not interrupt playback.
    fn notify(&self, event: Event) {
        if let Some(event_tx) = &self.event_tx {
            if let Err(e) = event_tx.send(event) {
                error!("failed to send event: {e}");
            }
        }
    }

    /// Registers an event notification channel.
    ///
    /// Events sent include:
    /// * Play/Pause state changes
    /// * Track changes
    /// * Connection status
    pub fn register(&mut self, event_tx: tokio::sync::mpsc::UnboundedSender<Event>) {
        self.event_tx = Some(event_tx);
    }

    /// Returns a mutable reference to the sink if available.
    ///
    /// # Errors
    /// Returns error if audio device is not open.
    fn sink_mut(&mut self) -> Result<&mut rodio::Sink> {
        self.sink
            .as_mut()
            .ok_or_else(|| Error::unavailable("audio sink not available"))
    }

    /// Starts or resumes playback.
    ///
    /// If audio device is not yet opened, opens it automatically.
    /// Emits a Play event if playback actually starts.
    /// Does nothing if already playing.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Audio device fails to open
    /// * Device is no longer available
    pub fn play(&mut self) -> Result<()> {
        // Ensure the audio device is open.
        self.start()?;

        if !self.is_playing() {
            debug!("starting playback");
            let pos = {
                let sink_mut = self.sink_mut()?;
                sink_mut.play();
                sink_mut.get_pos()
            };

            // Reset the playback start time for live streams.
            if self.track().is_some_and(Track::is_livestream) {
                self.playing_since = pos;
            }

            // Playback reporting happens every time a track starts playing or is unpaused.
            self.notify(Event::Play);
        }
        Ok(())
    }

    /// Pauses playback.
    ///
    /// Emits a Pause event if playback was actually playing.
    /// Does nothing if already paused.
    ///
    /// # Errors
    ///
    /// Returns error if audio device is not open.
    pub fn pause(&mut self) {
        if self.is_playing() {
            debug!("pausing playback");
            // Don't care if the sink is already dropped: we're already "paused".
            let _ = self.sink_mut().map(|sink| sink.pause());
            self.notify(Event::Pause);
        }
    }

    /// Returns whether playback is active.
    ///
    /// # Returns
    ///
    /// `true` if both:
    /// * A track is loaded (`current_rx` is Some)
    /// * Audio device is open and sink is not paused
    ///
    /// Note: Will return `false` if audio device is not open,
    /// even if a track is loaded and ready to play.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.current_rx.is_some() && self.sink.as_ref().is_some_and(|sink| !sink.is_paused())
    }

    /// Sets the playback state.
    ///
    /// Convenience method that:
    /// * Calls `play()` if `should_play` is true
    /// * Calls `pause()` if `should_play` is false
    ///
    /// # Arguments
    ///
    /// * `should_play` - Desired playback state
    ///
    /// # Errors
    ///
    /// Returns error if audio device is not open.
    pub fn set_playing(&mut self, should_play: bool) -> Result<()> {
        if should_play {
            self.play()
        } else {
            self.pause();
            Ok(())
        }
    }

    /// Returns the currently playing track, if any.
    #[must_use]
    #[inline]
    pub fn track(&self) -> Option<&Track> {
        self.queue.get(self.position)
    }

    /// Returns a mutable reference to the currently playing track, if any.
    #[must_use]
    #[inline]
    pub fn track_mut(&mut self) -> Option<&mut Track> {
        self.queue.get_mut(self.position)
    }

    /// Replaces the entire playback queue.
    ///
    /// * Clears current queue and playback state
    /// * Sets queue to the provided track order
    /// * Resets position to start
    /// * Clears skip track list
    pub fn set_queue(&mut self, tracks: Vec<Track>) {
        self.clear();
        self.position = 0;
        self.queue = tracks;
        self.skip_tracks = HashSet::new();
    }

    /// Returns a reference to the next track in the queue, if any.
    #[must_use]
    #[inline]
    pub fn next_track(&self) -> Option<&Track> {
        let next = self.position.saturating_add(1);
        self.queue.get(next)
    }

    /// Returns a mutable reference to the next track in the queue, if any.
    #[must_use]
    #[inline]
    pub fn next_track_mut(&mut self) -> Option<&mut Track> {
        let next = self.position.saturating_add(1);
        self.queue.get_mut(next)
    }

    /// Reorders the playback queue according to given track IDs.
    ///
    /// # Arguments
    ///
    /// * `track_ids` - New ordered list of track IDs
    ///
    /// This function:
    /// * Maintains the currently playing track
    /// * Reorders remaining tracks to match provided order
    /// * Updates internal queue position
    /// * Clears preloaded tracks to reflect new order
    pub fn reorder_queue(&mut self, track_ids: &[TrackId]) {
        let current_track_id = self.track().map(Track::id);
        let next_track_id = self.next_track().map(Track::id);

        // Reorder the queue based on the new track order.
        let mut new_queue = Vec::with_capacity(track_ids.len());
        for new_track_id in track_ids {
            if let Some(position) = self
                .queue
                .iter()
                .position(|track| &track.id() == new_track_id)
            {
                let mut new_track = self.queue.remove(position);

                // Reset the download state of tracks that are not in the current or next position.
                if ![current_track_id, next_track_id].contains(&Some(new_track.id())) {
                    new_track.reset_download();
                }

                new_queue.push(new_track);
            }
        }

        // Find the new position of the current track in the new queue.
        self.position = new_queue
            .iter()
            .position(|track| Some(track.id()) == current_track_id)
            .unwrap_or_default();

        // Set the new queue and clear the current track and preloaded track.
        self.queue = new_queue;
        self.preload_rx = None;
        self.sources.as_mut().map(|sources| sources.clear());
    }

    /// Adds tracks to the end of the queue.
    ///
    /// Preserves current playback position and state.
    pub fn extend_queue(&mut self, tracks: Vec<Track>) {
        self.queue.extend(tracks);
    }

    /// Sets the current playback position in the queue.
    ///
    /// Position can exceed queue length to prepare for
    /// future queue updates.
    ///
    /// Note: Setting to current position is ignored to
    /// prevent interrupting seeks.
    pub fn set_position(&mut self, position: usize) {
        // If the position is already set, do nothing. Deezer also sends the same position when
        // seeking, in which case we should not clear the current track.
        if self.position == position {
            return;
        }

        info!("setting playlist position to {position}");

        // Clear the sink, which will drop any handles to the current and next tracks.
        self.clear();
        self.position = position;
    }

    /// Clears the playback state.
    ///
    /// When sink is active:
    /// * Applies a short fade-out ramp to prevent audio popping
    /// * Drains output queue gracefully
    /// * Creates new empty source queue
    /// * Restores original volume after fade
    /// * Maintains playback state
    ///
    /// Also:
    /// * Resets track downloads
    /// * Resets internal playback state (position, receivers)
    pub fn clear(&mut self) {
        // Apply a short fade-out to prevent popping.
        let original_volume = self.ramp_volume(0.0);

        if let Ok(sink) = self.sink_mut() {
            // Don't clear the sink, because that makes Rodio:
            // - drop the entire output queue
            // - pause playback
            //
            // Instead, signal Rodio to stop which will make it:
            // - drain the output queue (preventing stale audio from playing)
            // - keep the playback state
            //
            // Because all sources are dropped, any downloads in progress will be cancelled.
            sink.stop();

            // Restore the original volume, if any.
            let _ = original_volume.inspect(|volume| sink.set_volume(*volume));

            // With Rodio having dropped the previous output queue, we need to create a new one.
            let (sources, output) = rodio::queue::queue(true);
            sink.append(output);
            self.sources = Some(sources);
        }

        // Resetting the sink drops any downloads of the current and next tracks.
        // We need to reset the download state of those tracks.
        if let Some(current) = self.track_mut() {
            current.reset_download();
        }
        if let Some(next) = self.next_track_mut() {
            next.reset_download();
        }

        self.playing_since = Duration::ZERO;
        self.current_rx = None;
        self.preload_rx = None;
    }

    /// Returns the current repeat mode.
    #[must_use]
    #[inline]
    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    /// Sets the repeat mode for playback.
    ///
    /// When setting to `RepeatMode::One`:
    /// * Clears preloaded track
    /// * Disables track preloading
    pub fn set_repeat_mode(&mut self, repeat_mode: RepeatMode) {
        info!("setting repeat mode to {repeat_mode}");
        self.repeat_mode = repeat_mode;

        if repeat_mode == RepeatMode::One {
            // This only clears the preloaded track.
            self.sources.as_mut().map(|sources| sources.clear());
            self.preload_rx = None;
        }
    }

    /// Returns the last volume setting as a percentage.
    ///
    /// Returns the raw volume value that was set, before logarithmic scaling is applied.
    /// The actual audio output uses logarithmic scaling to match human perception.
    ///
    /// # Returns
    ///
    /// * The last volume set via `set_volume()`
    /// * 1.0 (100%) if volume was never set
    ///
    /// Note: This returns the stored volume setting even if the audio device is closed.
    #[must_use]
    #[inline]
    pub fn volume(&self) -> Percentage {
        self.volume
    }

    /// Sets playback volume with logarithmic scaling.
    ///
    /// The volume control uses a logarithmic scale that matches human perception:
    /// * Logarithmic scaling across a 60 dB dynamic range
    /// * Linear fade to zero for very low volumes (< 10%)
    /// * Smooth transitions across the entire range
    /// * Gradual volume ramping to prevent audio popping
    ///
    /// No effect if new volume equals current volume.
    ///
    /// # Returns
    ///
    /// Returns the previous volume, wrapped in `Result`.
    ///
    /// # Arguments
    ///
    /// * `target` - Target volume percentage (0.0 to 1.0)
    ///
    /// # Errors
    ///
    /// Returns error if audio device is not open.
    pub fn set_volume(&mut self, target: Percentage) -> Result<Percentage> {
        // Check if the volume is already set to the target value:
        // Deezer sends the same volume on every status update, even if it hasn't changed.
        let current = self.volume();
        if target == current {
            return Ok(current);
        }

        info!("setting volume to {target}");

        // Clamp just in case the volume is set outside the valid range.
        let volume = target.as_ratio_f32().clamp(0.0, UNITY_GAIN);
        let mut amplitude = volume;

        // Apply logarithmic volume scaling with a smooth transition to zero.
        // Source: https://www.dr-lex.be/info-stuff/volumecontrols.html
        if amplitude > 0.0 && amplitude < UNITY_GAIN {
            amplitude =
                f32::exp(Self::LOG_VOLUME_GROWTH_RATE * volume) / Self::LOG_VOLUME_SCALE_FACTOR;
            if volume < 0.1 {
                amplitude *= volume * 10.0;
            }
            debug!(
                "volume scaled logarithmically to {}",
                Percentage::from_ratio_f32(amplitude)
            );
        }

        match self.ramp_volume(amplitude).map(Percentage::from_ratio_f32) {
            Ok(previous) => {
                self.volume = target;
                Ok(previous)
            }
            Err(e) => Err(e),
        }
    }

    /// Gradually changes audio volume over a short duration to prevent popping.
    ///
    /// Applies a linear volume ramp between the current and target volumes over
    /// `FADE_DURATION` milliseconds. This prevents audio artifacts that can occur
    /// with sudden volume changes.
    ///
    /// # Arguments
    ///
    /// * `target` - Target volume level (0.0 to 1.0)
    ///
    /// # Returns
    ///
    /// Returns the original volume before ramping, wrapped in `Result`.
    ///
    /// # Errors
    ///
    /// Returns error if audio device is not open.
    ///
    /// # Implementation Note
    ///
    /// Uses thread sleep for timing rather than async to ensure precise volume
    /// transitions. The short sleep duration (25ms total) makes this acceptable.
    fn ramp_volume(&mut self, target: f32) -> Result<f32> {
        let sink_mut = self.sink_mut()?;
        let original_volume = sink_mut.volume();

        let millis = Self::FADE_DURATION.as_millis();
        let fade_step = (target - original_volume) / millis.to_f32_lossy();

        for i in 1..=millis {
            let faded_volume = if i == millis {
                target
            } else {
                original_volume + fade_step * i.to_f32_lossy()
            };

            sink_mut.set_volume(faded_volume);

            // This blocks the current thread for 1 ms, but is better than making the
            // function async and waiting for the future to complete.
            std::thread::sleep(Duration::from_millis(1));
        }

        Ok(original_volume)
    }

    /// Returns current playback progress.
    ///
    /// Returns None if no track is playing or track duration is unknown.
    /// Progress is calculated as:
    /// * Regular tracks: Current position relative to total duration
    /// * Livestreams: Always reports 100% since they are continuous
    #[must_use]
    pub fn progress(&self) -> Option<Percentage> {
        self.track().and_then(|track| {
            // Livestreams are continuous and have no fixed duration.
            // We report 100% progress to indicate that they are always at the end.
            if track.is_livestream() {
                Some(Percentage::ONE_HUNDRED)
            } else {
                // The progress is the difference between the current position of the sink, which is the total duration played, and the time the current track started playing.
                let duration = track.duration()?;
                let progress = self.get_pos().saturating_sub(self.playing_since);
                Some(Percentage::from_ratio_f32(
                    progress.div_duration_f32(duration),
                ))
            }
        })
    }

    /// Returns duration of current track.
    ///
    /// For normal tracks, returns total duration.
    /// For livestreams, returns current stream duration since start.
    /// Returns None if no track or duration cannot be determined.
    pub fn duration(&self) -> Option<Duration> {
        self.track().and_then(|track| {
            if track.is_livestream() {
                self.sink
                    .as_ref()
                    .map(|sink| sink.get_pos().saturating_sub(self.playing_since))
            } else {
                track.duration()
            }
        })
    }

    /// Sets playback position within current track.
    ///
    /// # Behavior
    ///
    /// * If progress < 1.0:
    ///   - Seeks within track with proper logging of target position
    ///   - If position is beyond buffered data, seeks to last buffered position with warning
    ///   - Aligns seek to previous frame boundary for clean decoding
    ///   - Defers seek if track is not yet loaded
    /// * If progress >= 1.0: Skips to next track
    ///
    /// # Arguments
    ///
    /// * `progress` - Target position as percentage (0.0 to 1.0) of track duration
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No track is playing
    /// * Track duration cannot be determined
    /// * Audio device is not open
    /// * Seek operation fails (except for buffering/implementation limitations)
    pub fn set_progress(&mut self, progress: Percentage) -> Result<()> {
        if let Some(track) = self.track() {
            let duration = track.duration().ok_or_else(|| {
                Error::unavailable(format!("duration unknown for {} {track}", track.typ()))
            })?;

            let ratio = progress.as_ratio_f32();
            if ratio < 1.0 {
                let mut position = duration.mul_f32(ratio);
                let minutes = position.as_secs() / 60;
                let seconds = position.as_secs() % 60;
                info!(
                    "seeking {} {track} to {minutes:02}:{seconds:02} ({progress})",
                    track.typ()
                );

                // If the requested position is beyond what is buffered, seek to the buffered
                // position instead. This prevents blocking the player and disconnections.
                if let Some(buffered) = track.buffered() {
                    if duration > buffered {
                        if position > buffered {
                            position = buffered;
                        }

                        // Seek to just before the requested position, to be sure that we find the
                        // frame just before it. This helps prevents decoder errors.
                        if let Some(frame_duration) = track.codec().map(|codec| {
                            codec.max_frame_duration(
                                track.sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE),
                                track.channels.unwrap_or(track.typ().default_channels()),
                            )
                        }) {
                            position = position.saturating_sub(frame_duration);
                        }

                        let minutes = position.as_secs() / 60;
                        let seconds = position.as_secs() % 60;
                        warn!("limiting seek to {minutes:02}:{seconds:02} due to buffering");
                    }
                }

                // Try to seek only if the track has started downloading, otherwise defer the seek.
                // This prevents stalling the player when seeking in a track that has not started.
                match track
                    .handle()
                    .ok_or_else(|| {
                        Error::unavailable(format!(
                            "download of {} {track} not yet started",
                            track.typ()
                        ))
                    })
                    .and_then(|_| {
                        self.sink_mut()
                            .and_then(|sink| sink.try_seek(position).map_err(Into::into))
                    }) {
                    Ok(()) => {
                        // Reset the playing time to zero, as the sink will now reset it also.
                        self.playing_since = Duration::ZERO;
                        self.deferred_seek = None;
                    }
                    Err(e) => {
                        if matches!(e.kind, ErrorKind::Unavailable | ErrorKind::Unimplemented) {
                            // If the current track is not buffered yet, we can't seek.
                            // In that case, we defer the seek until the track is buffered.
                            self.deferred_seek = Some(position);
                        } else {
                            // If the seek failed for any other reason, we return an error.
                            return Err(e);
                        }
                    }
                }
            } else {
                // Setting the progress to 1.0 is equivalent to skipping to the next track.
                // This prevents `UnexpectedEof` when seeking to the end of the track.
                info!(
                    "seeking {} {track} to end: skipping to next track",
                    track.typ()
                );
                self.clear();
                self.go_next();
            }
        }

        Ok(())
    }

    /// Returns current position in the queue.
    #[must_use]
    #[inline]
    pub fn position(&self) -> usize {
        self.position
    }

    /// Sets the license token for media access.
    #[inline]
    pub fn set_license_token(&mut self, license_token: impl Into<String>) {
        self.license_token = license_token.into();
    }

    /// Enables or disables volume normalization.
    #[inline]
    pub fn set_normalization(&mut self, normalization: bool) {
        self.normalization = normalization;
    }

    /// Sets target gain for volume normalization.
    ///
    /// Logs info message if normalization is enabled.
    ///
    /// # Arguments
    ///
    /// * `gain_target_db` - Target gain in decibels
    pub fn set_gain_target_db(&mut self, gain_target_db: i8) {
        if self.normalization {
            info!("normalizing volume to {gain_target_db} dB");
        }
        self.gain_target_db = gain_target_db;
    }

    /// Sets preferred audio quality for playback.
    ///
    /// Note: Actual quality may be lower if track is not
    /// available in requested quality.
    #[inline]
    pub fn set_audio_quality(&mut self, quality: AudioQuality) {
        self.audio_quality = quality;
    }

    /// Returns whether volume normalization is enabled.
    #[must_use]
    #[inline]
    pub fn normalization(&self) -> bool {
        self.normalization
    }

    /// Returns current license token.
    #[must_use]
    #[inline]
    pub fn license_token(&self) -> &str {
        &self.license_token
    }

    /// Returns current preferred audio quality setting.
    #[must_use]
    #[inline]
    pub fn audio_quality(&self) -> AudioQuality {
        self.audio_quality
    }

    /// Returns current normalization target gain.
    #[must_use]
    #[inline]
    pub fn gain_target_db(&self) -> i8 {
        self.gain_target_db
    }

    /// Sets the media content URL.
    #[inline]
    pub fn set_media_url(&mut self, url: Url) {
        self.media_url = url;
    }

    /// Returns whether the audio device is open.
    ///
    /// True if `start()` has been called and the device was successfully opened.
    /// False if device has not been opened or has been closed with `stop()`.
    ///
    /// # Example
    /// ```
    /// let mut player = Player::new(&config, "").await?;
    /// assert!(!player.is_started());
    ///
    /// player.start()?;
    /// assert!(player.is_started());
    ///
    /// player.stop();
    /// assert!(!player.is_started());
    /// ```
    #[must_use]
    #[inline]
    pub fn is_started(&self) -> bool {
        self.sink.is_some()
    }
}

/// Ensures proper cleanup of audio device resources when player is dropped.
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}
