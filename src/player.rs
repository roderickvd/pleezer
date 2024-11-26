use std::{collections::HashSet, sync::Arc, time::Duration};

use cpal::traits::{DeviceTrait, HostTrait};
use md5::{Digest, Md5};
use rodio::Source;

use crate::{
    config::Config,
    decrypt::{Decrypt, Key},
    error::{Error, Result},
    events::Event,
    http,
    protocol::{
        connect::{
            contents::{AudioQuality, RepeatMode},
            Percentage,
        },
        gateway,
        media::DEFAULT_MEDIA_URL,
    },
    track::{Track, TrackId},
};

/// The sample format used by the player, as determined by the decoder.
type SampleFormat = <rodio::decoder::Decoder<std::fs::File> as Iterator>::Item;

pub struct Player {
    /// The *preferred* audio quality. The actual quality may be lower if the
    /// track is not available in the preferred quality.
    audio_quality: AudioQuality,

    /// The license token to use for downloading tracks.
    license_token: String,

    /// The decryption key to use for decrypting tracks.
    bf_secret: Key,

    /// The track queue, a.k.a. the playlist.
    queue: Vec<Track>,

    /// The set of tracks to skip.
    skip_tracks: HashSet<TrackId>,

    /// The current position in the queue.
    position: usize,

    /// The position in the current track to seek to after it has been loaded.
    deferred_seek: Option<Duration>,

    /// The HTTP client to use for downloading tracks.
    client: http::Client,

    /// The repeat mode.
    repeat_mode: RepeatMode,

    /// Whether the playlist should be shuffled.
    shuffle: bool,

    /// Whether to normalize the audio.
    normalization: bool,

    /// The target volume to normalize to in dB.
    gain_target_db: i8,

    /// The channel to send playback events to.
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<Event>>,

    /// The audio output sink.
    sink: rodio::Sink,

    /// The source queue with the audio data.
    sources: Arc<rodio::queue::SourcesQueueInput<SampleFormat>>,

    /// The point in time of the sink when the current track started playing.
    playing_since: Duration,

    /// The signal to receive when the current track has finished playing.
    current_rx: Option<std::sync::mpsc::Receiver<()>>,

    /// The signal to receive when the preloaded track will finish playing.
    preload_rx: Option<std::sync::mpsc::Receiver<()>>,

    /// The audio output stream. Although not used directly, this field must be retained to keep
    /// the sink alive.
    _stream: rodio::OutputStream,

    /// The URL to use for media requests.
    media_url: String,
}

impl Player {
    /// Creates a new `Player` with the given `Config`.
    ///
    /// # Errors
    ///
    /// Will return `Err` if no HTTP client can be built from the `Config`.
    pub async fn new(config: &Config, device: &str) -> Result<Self> {
        let client = http::Client::without_cookies(config)?;

        let bf_secret = if let Some(secret) = config.bf_secret {
            secret
        } else {
            debug!("no bf_secret specified, fetching one from the web player");
            Config::try_key(&client).await?
        };

        if format!("{:x}", Md5::digest(*bf_secret)) != Config::BF_SECRET_MD5 {
            return Err(Error::permission_denied("the bf_secret is not valid"));
        }

        let (sink, stream) = Self::open_sink(device)?;
        let (sources, output) = rodio::queue::queue(true);

        // The output source will output silence when the queue is empty.
        // That will cause the sink to start playing, so we need to pause it.
        sink.append(output);
        sink.pause();

        #[expect(clippy::cast_possible_truncation)]
        let gain_target_db = gateway::user_data::Gain::default().target as i8;

        Ok(Self {
            queue: Vec::new(),
            skip_tracks: HashSet::new(),
            position: 0,
            audio_quality: AudioQuality::default(),
            client,
            license_token: String::new(),
            media_url: DEFAULT_MEDIA_URL.to_string(),
            bf_secret,
            repeat_mode: RepeatMode::default(),
            shuffle: false,
            normalization: false,
            gain_target_db,
            event_tx: None,
            playing_since: Duration::ZERO,
            deferred_seek: None,
            current_rx: None,
            preload_rx: None,
            _stream: stream,
            sink,
            sources,
        })
    }

    fn open_sink(device: &str) -> Result<(rodio::Sink, rodio::OutputStream)> {
        let (stream, handle) = {
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

            let (stream, handle) = match components.next() {
                Some("") | None => rodio::OutputStream::try_from_device(&device)?,
                Some(rate) => {
                    let rate = rate.parse().map_err(|_| {
                        Error::invalid_argument(format!("invalid sample rate {rate}"))
                    })?;
                    let rate = cpal::SampleRate(rate);

                    let format = match components.next() {
                        Some("") | None => None,
                        other => other,
                    };

                    let config = device
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
                            })?;

                    rodio::OutputStream::try_from_device_config(&device, config)?
                }
            };

            info!(
                "audio output device: {} on {}",
                device.name().as_deref().unwrap_or("UNKNOWN"),
                host.id().name()
            );

            (stream, handle)
        };

        let sink = rodio::Sink::try_new(&handle)?;
        Ok((sink, stream))
    }

    /// The list of supported sample rates.
    ///
    /// This list is used to filter out unreasonable sample rates.
    /// Common sample rates in Hz:
    /// * 44100 - CD audio, most streaming services
    /// * 48000 - Professional digital audio, DVDs, most DAWs
    /// * 88200/96000 - High resolution audio
    /// * 176400/192000 - Studio quality
    /// * 352800/384000 - Ultra high definition audio
    const SAMPLE_RATES: [u32; 8] = [
        44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
    ];

    #[must_use]
    pub fn enumerate_devices() -> Vec<String> {
        let hosts = cpal::available_hosts();

        // Create a set to store the unique device names.
        // On Alsa hosts, the same device may otherwise be enumerated multiple times.
        let mut result = HashSet::new();

        // Get the default host, device and config.
        let default_host = cpal::default_host();
        let default_device = default_host.default_output_device();
        let default_config = default_device
            .as_ref()
            .and_then(|device| device.default_output_config().ok());

        // Enumerate all available hosts, devices and configs.
        for host in hosts
            .into_iter()
            .filter_map(|id| cpal::host_from_id(id).ok())
        {
            if let Ok(devices) = host.output_devices() {
                for device in devices {
                    if let Ok(configs) = device.supported_output_configs() {
                        for config in configs {
                            if let Ok(device_name) = device.name() {
                                for sample_rate in &Self::SAMPLE_RATES {
                                    if let Some(config) =
                                        config.try_with_sample_rate(cpal::SampleRate(*sample_rate))
                                    {
                                        let mut line = format!(
                                            "{}|{}|{}|{}",
                                            host.id().name(),
                                            device_name,
                                            config.sample_rate().0,
                                            config.sample_format(),
                                        );

                                        // Check if this is the default host, device
                                        // and config.
                                        if default_host.id() == host.id()
                                            && default_device.as_ref().is_some_and(
                                                |default_device| {
                                                    default_device.name().is_ok_and(
                                                        |default_name| default_name == device_name,
                                                    )
                                                },
                                            )
                                            && default_config.as_ref().is_some_and(
                                                |default_config| *default_config == config,
                                            )
                                        {
                                            line.push_str(" (default)");
                                        }

                                        result.insert(line);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut result: Vec<_> = result.into_iter().collect();
        result.sort();
        result
    }

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

    /// The audio gain control (AGC) attack time.
    /// This value is equal to what Spotify uses.
    const AGC_ATTACK_TIME: Duration = Duration::from_millis(5);

    /// The audio gain control (AGC) release time.
    /// This value is equal to what Spotify uses.
    const AGC_RELEASE_TIME: Duration = Duration::from_millis(100);

    // TODO : consider controlflow
    async fn load_track(
        &mut self,
        position: usize,
    ) -> Result<Option<std::sync::mpsc::Receiver<()>>> {
        let track = self
            .queue
            .get_mut(position)
            .ok_or_else(|| Error::not_found(format!("track at position {position} not found")))?;

        if track.handle().is_none() {
            let download = tokio::time::timeout(Duration::from_secs(1), async {
                // Start downloading the track.
                let medium = track
                    .get_medium(
                        &self.client,
                        &self.media_url,
                        self.audio_quality,
                        self.license_token.clone(),
                    )
                    .await?;

                // Return `None` on success to indicate that the track is not yet appended
                // to the sink.
                track.start_download(&self.client, &medium).await
            })
            .await??;

            // Append the track to the sink.
            let decryptor = Decrypt::new(track, download, &self.bf_secret)?;
            let mut decoder = match track.quality() {
                AudioQuality::Lossless => rodio::Decoder::new_flac(decryptor),
                _ => rodio::Decoder::new_mp3(decryptor),
            }?;

            if let Some(progress) = self.deferred_seek.take() {
                // Set the track position only if `progress` is beyond the track start. We start
                // at the beginning anyway, and this prevents decoder errors.
                if !progress.is_zero() {
                    if let Err(e) = decoder.try_seek(progress) {
                        error!("failed to seek to deferred position: {}", e);
                    }
                }
            }

            let mut difference = 0.0;
            let mut ratio = 1.0;
            if self.normalization {
                match track.gain() {
                    Some(gain) => {
                        difference = f32::from(self.gain_target_db) - gain;

                        // Keep -1 dBTP of headroom on tracks with lossy decoding to avoid
                        // clipping due to inter-sample peaks.
                        if difference > 0.0 && !track.is_lossless() {
                            difference -= 1.0;
                        }

                        ratio = f32::powf(10.0, difference / 20.0);
                    }
                    None => {
                        warn!("track {track} has no gain information, skipping normalization");
                    }
                }
            }

            let rx = if ratio < 1.0 {
                debug!(
                    "attenuating track {track} by {difference:.1} dB ({})",
                    Percentage::from_ratio_f32(ratio)
                );
                let attenuated = decoder.amplify(ratio);
                self.sources.append_with_signal(attenuated)
            } else if ratio > 1.0 {
                debug!(
                    "amplifying track {track} by {difference:.1} dB ({}) (with limiter)",
                    Percentage::from_ratio_f32(ratio)
                );
                let amplified = decoder.automatic_gain_control(
                    ratio,
                    Self::AGC_ATTACK_TIME.as_secs_f32(),
                    Self::AGC_RELEASE_TIME.as_secs_f32(),
                    difference,
                );
                self.sources.append_with_signal(amplified)
            } else {
                self.sources.append_with_signal(decoder)
            };

            return Ok(Some(rx));
        }

        Ok(None)
    }

    /// Run the player.
    ///
    /// This function will monitor the position in the playlist and start downloading
    /// the track if it is pending. It will then play the track and skip to the next
    /// track when the current track is finished.
    ///
    /// # Errors
    ///
    /// This function may return an error if the player fails to start downloading
    /// the track, or if the player fails to play the track.
    pub async fn run(&mut self) -> Result<()> {
        loop {
            match self.current_rx.as_mut() {
                Some(current_rx) => {
                    // Check if the current track has finished playing.
                    if current_rx.try_recv().is_ok() {
                        // Save the point in time when the track finished playing.
                        self.playing_since = self.playing_since.saturating_add(self.sink.get_pos());

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
                            if !self.skip_tracks.contains(&next_track_id) {
                                match self.load_track(next_position).await {
                                    Ok(rx) => {
                                        self.preload_rx = rx;
                                    }
                                    Err(e) => {
                                        error!("failed to preload next track: {e}");
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
                        if self.skip_tracks.contains(&track_id) {
                            self.go_next();
                        } else {
                            match self.load_track(self.position).await {
                                Ok(rx) => {
                                    if let Some(rx) = rx {
                                        self.current_rx = Some(rx);
                                        self.notify(Event::TrackChanged);
                                    }
                                }
                                Err(e) => {
                                    error!("failed to load track: {e}");
                                    self.mark_unavailable(track_id);
                                    self.go_next();
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

    fn mark_unavailable(&mut self, track_id: TrackId) {
        if self.skip_tracks.insert(track_id) {
            warn!("marking track {track_id} as unavailable");
        }
    }

    fn notify(&self, event: Event) {
        if let Some(event_tx) = &self.event_tx {
            if let Err(e) = event_tx.send(event) {
                error!("failed to send event: {e}");
            }
        }
    }

    pub fn register(&mut self, event_tx: tokio::sync::mpsc::UnboundedSender<Event>) {
        self.event_tx = Some(event_tx);
    }

    pub fn play(&mut self) {
        if !self.is_playing() {
            debug!("starting playback");
            self.sink.play();

            // Playback reporting happens every time a track starts playing or is unpaused.
            self.notify(Event::Play);
        }
    }

    pub fn pause(&mut self) {
        if self.is_playing() {
            debug!("pausing playback");
            self.sink.pause();
            self.notify(Event::Pause);
        }
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.current_rx.is_some() && !self.sink.is_paused()
    }

    pub fn set_playing(&mut self, should_play: bool) {
        if should_play {
            self.play();
        } else {
            self.pause();
        }
    }

    #[must_use]
    pub fn track(&self) -> Option<&Track> {
        self.queue.get(self.position)
    }

    pub fn set_queue(&mut self, tracks: Vec<Track>) {
        self.clear();
        self.position = 0;
        self.queue = tracks;
        self.skip_tracks.clear();
        self.skip_tracks.shrink_to_fit();
    }

    pub fn extend_queue(&mut self, tracks: Vec<Track>) {
        self.queue.extend(tracks);
    }

    /// Sets the playlist position.
    ///
    /// It is allowed to set the position to a value that is greater than the length of the queue.
    /// This is useful when the queue is not yet set, but the future position is already known.
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

    pub fn clear(&mut self) {
        // Don't just clear the sink, because that would stop the playback. The following code
        // works around that by creating a new, empty queue of sources and skipping to it.
        let (sources, output) = rodio::queue::queue(true);
        self.sink.append(output);
        self.sink.skip_one();
        self.sources = sources;

        // Resetting the sink drops any downloads of the current and next tracks.
        // We need to reset the download state of those tracks.
        if let Some(current) = self.queue.get_mut(self.position) {
            current.reset_download();
        }
        if let Some(next) = self.queue.get_mut(self.position.saturating_add(1)) {
            next.reset_download();
        }

        self.playing_since = Duration::ZERO;
        self.current_rx = None;
        self.preload_rx = None;
    }

    #[must_use]
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        info!("setting shuffle to {shuffle}");
        self.shuffle = shuffle;

        // TODO: implement shuffle
        if shuffle {
            warn!("shuffle is not yet implemented");
        }
    }

    #[must_use]
    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    pub fn set_repeat_mode(&mut self, repeat_mode: RepeatMode) {
        info!("setting repeat mode to {repeat_mode}");
        self.repeat_mode = repeat_mode;

        if repeat_mode == RepeatMode::One {
            // This only clears the preloaded track.
            self.sources.clear();
            self.preload_rx = None;
        }
    }

    #[must_use]
    pub fn volume(&self) -> Percentage {
        let ratio = self.sink.volume();
        Percentage::from_ratio_f32(ratio)
    }

    pub fn set_volume(&mut self, volume: Percentage) {
        if volume == self.volume() {
            return;
        }

        info!("setting volume to {volume}");
        let ratio = volume.as_ratio_f32();
        self.sink.set_volume(ratio);
    }

    #[must_use]
    pub fn progress(&self) -> Option<Percentage> {
        // The progress is the difference between the current position of the sink, which is the
        // total duration played, and the time the current track started playing.
        let progress = self.sink.get_pos().saturating_sub(self.playing_since);

        self.track().map(|track| {
            let ratio = progress.div_duration_f32(track.duration());
            Percentage::from_ratio_f32(ratio)
        })
    }

    /// # Errors
    ///
    /// Will return `Err` if:
    /// - there is no active track
    pub fn set_progress(&mut self, progress: Percentage) -> Result<()> {
        if let Some(track) = self.track() {
            info!("setting track progress to {progress}");
            let progress = progress.as_ratio_f32();
            if progress < 1.0 {
                let progress = track.duration().mul_f32(progress);
                match self.sink.try_seek(progress) {
                    Ok(()) => {
                        // Reset the playing time to zero, as the sink will now reset it also.
                        self.playing_since = Duration::ZERO;
                        self.deferred_seek = None;
                    }
                    Err(e) => {
                        if let rodio::source::SeekError::NotSupported { .. } = e {
                            // If the current track is not buffered yet, we can't seek.
                            // In that case, we defer the seek until the track is buffered.
                            self.deferred_seek = Some(progress);
                        } else {
                            // If the seek failed for any other reason, we return an error.
                            return Err(e.into());
                        }
                    }
                }
            } else {
                // Setting the progress to 1.0 is equivalent to skipping to the next track.
                // This prevents `UnexpectedEof` when seeking to the end of the track.
                self.clear();
                self.go_next();
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.position
    }

    pub fn set_license_token(&mut self, license_token: impl Into<String>) {
        self.license_token = license_token.into();
    }

    pub fn set_normalization(&mut self, normalization: bool) {
        self.normalization = normalization;
    }

    pub fn set_gain_target_db(&mut self, gain_target_db: i8) {
        self.gain_target_db = gain_target_db;
    }

    pub fn set_audio_quality(&mut self, quality: AudioQuality) {
        self.audio_quality = quality;
    }

    #[must_use]
    pub fn normalization(&self) -> bool {
        self.normalization
    }

    #[must_use]
    pub fn license_token(&self) -> &str {
        &self.license_token
    }

    #[must_use]
    pub fn audio_quality(&self) -> AudioQuality {
        self.audio_quality
    }

    #[must_use]
    pub fn gain_target_db(&self) -> i8 {
        self.gain_target_db
    }

    pub fn set_media_url(&mut self, url: &str) {
        self.media_url = url.to_string();
    }
}
