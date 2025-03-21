//! Remote control protocol implementation for Deezer Connect.
//!
//! This module implements the client side of the Deezer Connect protocol,
//! enabling remote control functionality between devices. It handles:
//! * Device discovery and connection
//! * Authentication and session management
//! * Optional JWT login for enhanced features
//! * Session persistence and automatic renewal
//! * Command processing
//! * Queue synchronization and manipulation
//! * Playback reporting
//! * Event notifications
//!
//! # Hook Scripts
//!
//! Hook scripts can be configured to execute on events, receiving information
//! through environment variables. All events include the `EVENT` variable
//! containing the event name, plus additional variables specific to each event:
//!
//! ## `playing`
//! Emitted when playback starts
//!
//! Variables:
//! - `TRACK_ID`: The ID of the track being played
//!
//! ## `paused`
//! Emitted when playback is paused
//!
//! No additional variables
//!
//! ## `track_changed`
//! Emitted when the track changes
//!
//! Common variables for all content:
//! - `TRACK_TYPE`: Content type ("song", "episode", "livestream")
//! - `TRACK_ID`: Content identifier
//! - `ARTIST`: Artist name/podcast title/station name
//! - `COVER_ID`: Cover art identifier
//! - `FORMAT`: Input format and bitrate (e.g. "MP3 320K", "FLAC 1.234M")
//! - `DECODER`: Decoded format including:
//!   * Sample format ("PCM 16/24/32 bit")
//!   * Sample rate (e.g. "44.1 kHz")
//!   * Channel configuration (e.g. "Stereo")
//!
//! Additional variables for songs and episodes:
//! - `TITLE`: Track/episode title
//! - `DURATION`: Length in seconds
//!
//! Additional variables for songs:
//! - `ALBUM_TITLE`: Album name
//!
//! ## `connected`
//! Emitted when a controller connects
//!
//! Variables:
//! - `USER_ID`: The Deezer user ID
//! - `USER_NAME`: The Deezer username
//!
//! ## `disconnected`
//! Emitted when the controller disconnects
//!
//! No additional variables
//!
//! # Protocol Details
//!
//! ## Connection Flow
//!
//! 1. Connection Establishment
//!    * Client connects to Deezer websocket
//!    * Authenticates with user token
//!    * Subscribes to required channels
//!
//! 2. Device Discovery
//!    * Client announces availability
//!    * Controllers send discovery requests with session IDs
//!    * Client caches session IDs and responds once per session
//!    * Client responds with connection offers
//!
//! 3. Control Session
//!    * Controller initiates connection
//!    * Client accepts if available
//!    * Establishes session and channel subscriptions
//!    * Commands flow between devices
//!    * Queue and playback state synchronized (including shuffle)
//!
//! ## Connection States
//!
//! A client progresses through several states:
//! * Disconnected - Initial state
//! * Available - Ready for discovery
//! * Connecting - Accepting controller
//! * Connected - Active control session
//! * Taken - Connection locked (if interruptions disabled)
//!
//! ## Message Types
//!
//! The protocol uses several message types:
//! * Discovery - Device detection
//! * Command - Playback control
//! * Queue - Content management
//! * Stream - Playback reporting
//! * Status - Command acknowledgement
//!
//! # Example
//!
//! ```rust
//! use pleezer::remote::Client;
//!
//! let mut client = Client::new(&config, player)?;
//!
//! // Start client and handle control messages
//! client.start().await?;
//! ```

use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    ops::ControlFlow,
    pin::Pin,
    process::Command,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use log::Level;
use semver;
use time::OffsetDateTime;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{
        Message as WebsocketMessage,
        client::ClientRequestBuilder,
        protocol::{WebSocketConfig, frame::Frame},
    },
};
use uuid::Uuid;

use crate::{
    config::{Config, Credentials},
    error::{Error, Result},
    events::Event,
    gateway::Gateway,
    player::Player,
    protocol::connect::{
        Body, Channel, Contents, DeviceId, DeviceType, Headers, Ident, Message, Percentage,
        QueueItem, RepeatMode, Status, UserId,
        queue::{self, MixType},
        stream,
    },
    proxy,
    tokens::UserToken,
    track::{DEFAULT_BITS_PER_SAMPLE, DEFAULT_SAMPLE_RATE, Track, TrackId},
    util::ToF32,
};

/// A client on the Deezer Connect protocol.
pub struct Client {
    /// Unique identifier for this device
    device_id: DeviceId,

    /// Human-readable device name shown in discovery
    device_name: String,

    /// Device type identifier (mobile, desktop, etc)
    device_type: DeviceType,

    /// User authentication credentials
    credentials: Credentials,

    /// Gateway API client
    gateway: Gateway,

    /// Current user authentication token
    user_token: Option<UserToken>,

    /// Channel for token lifetime updates
    time_to_live_tx: tokio::sync::mpsc::Sender<Duration>,

    /// Receiver for token lifetime updates
    time_to_live_rx: tokio::sync::mpsc::Receiver<Duration>,

    /// Protocol version string
    version: String,

    /// Websocket message sender
    websocket_tx:
        Option<SplitSink<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, WebsocketMessage>>,

    /// Active channel subscriptions
    subscriptions: HashSet<Ident>,

    /// Current connection state
    connection_state: ConnectionState,

    /// Timer for receiving controller heartbeats
    watchdog_rx: Pin<Box<tokio::time::Sleep>>,

    /// Timer for sending heartbeats
    watchdog_tx: Pin<Box<tokio::time::Sleep>>,

    /// Current discovery state
    discovery_state: DiscoveryState,

    /// Cache of discovery session IDs to prevent duplicate offers within a single connection
    ///
    /// Maps controller device IDs to their current discovery session ID. Cleared when client
    /// starts/restarts to prevent memory exhaustion across reconnections. This caches by
    /// device rather than session since the same controllers typically reconnect multiple times.
    discovery_sessions: HashMap<DeviceId, String>,

    /// Channel for receiving player and control events
    event_rx: tokio::sync::mpsc::UnboundedReceiver<Event>,

    /// Channel for sending player and control events
    event_tx: tokio::sync::mpsc::UnboundedSender<Event>,

    /// Volume level to set on connection and maintain until client sets below maximum.
    /// Helps work around clients that don't properly set volume levels.
    initial_volume: InitialVolume,

    /// Whether to allow connection interruptions
    interruptions: bool,

    /// Optional hook script for events
    hook: Option<String>,

    /// Audio playback manager
    player: Player,

    /// Timer for playback progress reports
    reporting_timer: Pin<Box<tokio::time::Sleep>>,

    /// Current playback queue
    ///
    /// Maintains both track list and shuffle state.
    queue: Option<queue::List>,

    /// Position to set when queue arrives
    ///
    /// Used to handle position changes that arrive before queue.
    deferred_position: Option<usize>,

    /// Whether to monitor all websocket traffic
    eavesdrop: bool,
}

/// Device discovery state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DiscoveryState {
    /// Available for discovery
    Available,

    /// Accepting connection from controller
    Connecting {
        /// Controller device ID
        controller: DeviceId,

        /// ID of ready message
        ready_message_id: String,
    },

    /// Not available for discovery
    Taken,
}

/// Connection state with controller.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ConnectionState {
    /// No active connection
    Disconnected,

    /// Connected to controller
    Connected {
        /// Controller device ID
        controller: DeviceId,

        /// Unique session identifier
        session_id: Uuid,
    },
}

/// Direction for queue shuffling operations.
///
/// Controls whether to:
/// * `Shuffle` - Randomize track order
/// * `Unshuffle` - Restore original track order
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum ShuffleAction {
    /// Randomize track order
    Shuffle,
    /// Restore original track order
    Unshuffle,
}

/// Volume initialization state.
///
/// Controls how initial volume is applied:
/// * `Active` - Set volume and remain active until client sets below maximum
/// * `Inactive` - Initial volume has been superseded by client control
/// * `Disabled` - No initial volume configured
///
/// The state transitions from Active to Inactive when the client takes control by setting
/// a volume below maximum. When a connection ends, it transitions back to Active to ensure
/// the initial volume is reapplied on reconnection.
#[derive(Copy, Clone, Debug, PartialEq)]
enum InitialVolume {
    /// Initial volume is active and will be applied on connection/reconnection
    Active(Percentage),
    /// Initial volume is stored but inactive (client has taken control)
    Inactive(Percentage),
    /// No initial volume configured
    Disabled,
}

/// Calculates a future time instant by adding seconds to now.
///
/// Used for scheduling timers and watchdogs. Handles overflow
/// by returning None if addition would overflow.
///
/// # Arguments
///
/// * `seconds` - Duration to add to current time
///
/// # Returns
///
/// * `Some(Instant)` - Future time if addition succeeds
/// * `None` - If addition would overflow
#[must_use]
#[inline]
fn from_now(seconds: Duration) -> Option<tokio::time::Instant> {
    tokio::time::Instant::now().checked_add(seconds)
}

/// A client on the Deezer Connect protocol.
///
/// Handles:
/// * Device discovery and connections
/// * Command processing
/// * Queue management
/// * Playback state synchronization
/// * Volume management and normalization
/// * Event notifications
impl Client {
    /// Time before network operations timeout.
    const NETWORK_TIMEOUT: Duration = Duration::from_secs(2);

    /// Buffer before token refresh to prevent expiration during requests.
    const TOKEN_EXPIRATION_THRESHOLD: Duration = Duration::from_secs(60);

    /// How often to report playback progress to controller.
    const REPORTING_INTERVAL: Duration = Duration::from_secs(3);

    /// Maximum time to wait for controller heartbeat.
    const WATCHDOG_RX_TIMEOUT: Duration = Duration::from_secs(10);

    /// Maximum time between sending heartbeats.
    const WATCHDOG_TX_TIMEOUT: Duration = Duration::from_secs(5);

    /// Maximum allowed websocket frame size (payload) in bytes.
    /// Set to 32KB (message size / 4) to balance between chunking and overhead.
    const FRAME_SIZE_MAX: usize = Self::MESSAGE_SIZE_MAX / 4;

    /// Maximum allowed websocket message size (payload plus headers) in bytes.
    /// Set to 128KB (message buffer / 2) to provide backpressure and prevent OOM.
    const MESSAGE_SIZE_MAX: usize = Self::MESSAGE_BUFFER_MAX / 2;

    /// Maximum size of the websocket write buffer in bytes.
    /// Set to 256KB to provide adequate buffering while preventing memory exhaustion.
    const MESSAGE_BUFFER_MAX: usize = 2 * 128 * 1024;

    /// Default session TTL (4 hours)
    const SESSION_DEFAULT_TTL: Duration = Duration::from_secs(4 * 3600);

    /// Cookie name to get session expiration from
    const SESSION_COOKIE_NAME: &'static str = "bm_sz";

    /// Default JWT TTL (30 days)
    const JWT_DEFAULT_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

    /// Cookie name to get JWT expiration from
    const JWT_COOKIE_NAME: &'static str = "refresh-token";

    /// Deezer Connect websocket URL.
    const WEBSOCKET_URL: &'static str = "wss://live.deezer.com/ws/";

    /// Creates a new client instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration including device and authentication settings
    /// * `player` - Audio playback manager instance
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Application version in config is not valid `SemVer`
    /// * Gateway client creation fails
    pub fn new(config: &Config, player: Player) -> Result<Self> {
        // Construct version in the form of `Mmmppp` where:
        // - `M` is the major version
        // - `mm` is the minor version
        // - `ppp` is the patch version
        let semver = semver::Version::parse(&config.app_version)?;
        let major = semver.major;
        let minor = semver.minor;
        let patch = semver.patch;

        // Trim leading zeroes.
        let version = if major > 0 {
            format!("{major}{minor:0>2}{patch:0>3}")
        } else if minor > 0 {
            format!("{minor}{patch:0>3}")
        } else {
            format!("{patch}")
        };
        trace!("remote version: {version}");

        // Timers are set in the message handlers. They should be moved into
        // a state variant once `select!` supports `if let` statements:
        // https://github.com/tokio-rs/tokio/issues/4173
        let reporting_timer = tokio::time::sleep(Duration::ZERO);
        let watchdog_rx = tokio::time::sleep(Duration::ZERO);
        let watchdog_tx = tokio::time::sleep(Duration::ZERO);

        let (time_to_live_tx, time_to_live_rx) = tokio::sync::mpsc::channel(1);
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();

        let mut player = player;
        player.register(event_tx.clone());

        let initial_volume = match config.initial_volume {
            Some(volume) => InitialVolume::Active(volume),
            None => InitialVolume::Disabled,
        };

        Ok(Self {
            device_id: config.device_id.into(),
            device_name: config.device_name.clone(),
            device_type: config.device_type,

            credentials: config.credentials.clone(),
            gateway: Gateway::new(config)?,

            user_token: None,
            time_to_live_tx,
            time_to_live_rx,

            version,
            websocket_tx: None,

            subscriptions: HashSet::new(),

            connection_state: ConnectionState::Disconnected,
            watchdog_rx: Box::pin(watchdog_rx),
            watchdog_tx: Box::pin(watchdog_tx),

            event_rx,
            event_tx,

            player,
            reporting_timer: Box::pin(reporting_timer),

            discovery_state: DiscoveryState::Available,
            discovery_sessions: HashMap::new(),

            initial_volume,
            interruptions: config.interruptions,
            hook: config.hook.clone(),

            queue: None,
            deferred_position: None,

            eavesdrop: config.eavesdrop,
        })
    }

    /// Retrieves a valid user token from the gateway.
    ///
    /// Repeatedly attempts to get a token that expires after the threshold.
    /// Returns both the token and its time-to-live for expiration tracking.
    ///
    /// # Returns
    ///
    /// Tuple containing:
    /// * `UserToken` - Valid authentication token
    /// * Duration - Time until token expires
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Gateway request fails
    /// * Token cannot be retrieved
    async fn user_token(&mut self) -> Result<(UserToken, Duration)> {
        // Loop until a user token is supplied that expires after the
        // threshold. If rate limiting is necessary, then that should be done
        // by the token token_provider.
        loop {
            let token =
                tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.user_token()).await??;

            let time_to_live = token
                .time_to_live()
                .checked_sub(Self::TOKEN_EXPIRATION_THRESHOLD);

            match time_to_live {
                Some(duration) => {
                    // This takes a few milliseconds and would normally
                    // truncate (round down). Return `ceil` is more human
                    // readable.
                    debug!(
                        "user data time to live: {:.0}s",
                        duration.as_secs_f32().ceil(),
                    );

                    break Ok((token, duration));
                }
                None => {
                    // Flush user tokens that expire within the threshold.
                    self.gateway.flush_user_token();
                }
            }
        }
    }

    /// Configures player settings from user preferences.
    ///
    /// Updates:
    /// * Audio quality
    /// * Volume normalization
    /// * License token
    /// * Media URL
    fn set_player_settings(&mut self) {
        let audio_quality = self.gateway.audio_quality();
        info!("user casting quality: {audio_quality}");
        self.player.set_audio_quality(audio_quality);

        let gain_target_db = self.gateway.target_gain();
        self.player.set_gain_target_db(gain_target_db);

        if let Some(license_token) = self.gateway.license_token() {
            self.player.set_license_token(license_token);
        }

        self.player.set_media_url(self.gateway.media_url());
    }

    /// Formats cookies from gateway for websocket connection.
    ///
    /// Extracts name-value pairs from cookies and formats them into a single string
    /// suitable for the Cookie HTTP header.
    ///
    /// # Returns
    ///
    /// Semicolon-separated list of "name=value" cookie pairs
    fn cookie_str(&self) -> String {
        let mut cookie_str = String::new();
        if let Some(cookies) = self.gateway.cookies() {
            for cookie in cookies.iter_unexpired() {
                if !cookie_str.is_empty() {
                    cookie_str.push(';');
                }
                let (name, value) = cookie.name_value();
                let _ = write!(cookie_str, "{name}={value}");
            }
        }

        cookie_str
    }

    /// Returns TTL for a specific cookie.
    ///
    /// Extracts expiration time from cookie metadata, handling both:
    /// * `max-age` attribute
    /// * `expires` attribute
    ///
    /// # Arguments
    ///
    /// * `name` - Name of cookie to check
    ///
    /// # Returns
    ///
    /// * `Some(Duration)` - Time until cookie expires
    /// * `None` - Cookie not found or already expired
    fn cookie_ttl(&self, name: &str) -> Option<Duration> {
        let mut cookie_ttl = None;
        if let Some(cookies) = self.gateway.cookies() {
            for cookie in cookies.iter_any() {
                if cookie.name() == name {
                    if let Some(max_age) = cookie.max_age().and_then(|ttl| ttl.try_into().ok()) {
                        cookie_ttl = Some(max_age);
                    } else if let Some(expires) = cookie.expires_datetime() {
                        let now = OffsetDateTime::now_utc();
                        if let Ok(ttl) = (expires - now).try_into() {
                            cookie_ttl = Some(ttl);
                        } else {
                            warn!("{name} expiry in the past: {expires}");
                        }
                    }
                }
            }
        }

        cookie_ttl
    }

    /// Returns TTL for session cookie, adjusted for token expiration threshold.
    ///
    /// Uses `bm_sz` cookie expiration or falls back to default session TTL of 4 hours.
    /// Subtracts expiration threshold to ensure timely renewal.
    ///
    /// # Returns
    ///
    /// Duration until session should be renewed
    fn session_ttl(&self) -> Duration {
        self.cookie_ttl(Self::SESSION_COOKIE_NAME)
            .unwrap_or(Self::SESSION_DEFAULT_TTL)
            .saturating_sub(Self::TOKEN_EXPIRATION_THRESHOLD)
    }

    /// Returns TTL for JWT cookie, adjusted for token expiration threshold.
    ///
    /// Uses `refresh-token` cookie expiration or falls back to default JWT TTL of 30 days.
    /// Subtracts expiration threshold to ensure timely renewal.
    ///
    /// # Returns
    ///
    /// Duration until JWT should be renewed
    fn jwt_ttl(&self) -> Duration {
        self.cookie_ttl(Self::JWT_COOKIE_NAME)
            .unwrap_or(Self::JWT_DEFAULT_TTL)
            .saturating_sub(Self::TOKEN_EXPIRATION_THRESHOLD)
    }

    /// Starts the client and handles control messages.
    ///
    /// Authentication flow:
    /// 1. Logs in with email/password or ARL to obtain refresh token
    /// 2. Gets user token using refresh token
    /// 3. Renews tokens automatically before expiration
    /// 4. Maintains persistent login across reconnects
    ///
    /// Connection flow:
    /// 1. Establishes websocket with configured limits
    /// 2. Authenticates connection
    /// 3. Clears cached discovery sessions
    /// 4. Begins message processing
    ///
    /// Processes:
    /// * Controller discovery
    /// * Command messages
    /// * Playback state updates
    /// * Connection maintenance
    /// * Token renewals
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Authentication fails
    /// * Websocket connection fails
    /// * Message handling fails critically
    /// * Token renewal fails
    #[allow(clippy::too_many_lines)]
    pub async fn start(&mut self) -> Result<()> {
        // Purge discovery sessions from any previous session to prevent memory exhaustion.
        self.discovery_sessions = HashMap::new();

        let arl = match self.credentials.clone() {
            Credentials::Login { email, password } => {
                info!("logging in with email and password");
                tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.oauth(&email, &password))
                    .await??
            }
            Credentials::Arl(arl) => {
                info!("using ARL from secrets file");
                arl
            }
        };

        // Soft failure: JWT logins are not required to interact with the gateway.
        match tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.login_with_arl(&arl)).await {
            Ok(inner) => {
                if let Err(e) = inner {
                    warn!("jwt login failed: {e}");
                } else {
                    debug!("jwt logged in");
                }
            }
            Err(e) => warn!("jwt login timed out: {e}"),
        }

        let (user_token, token_ttl) = self.user_token().await?;
        debug!("user id: {}", user_token.user_id);

        let uri = format!(
            "{}{}?version={}",
            Self::WEBSOCKET_URL,
            user_token,
            self.version
        );
        let mut request = ClientRequestBuilder::new(uri.parse::<http::Uri>()?);
        self.user_token = Some(user_token);

        // Decorate the websocket request with the same cookies as the gateway.
        let cookie_str = self.cookie_str();
        request = request.with_header(http::header::COOKIE.as_str(), cookie_str);

        // Set timer for user token expiration. Wake a short while before
        // actual expiration. This prevents API request errors when the
        // expiration is checked with only a few seconds on the clock.
        let token_expiry = tokio::time::sleep(token_ttl);
        tokio::pin!(token_expiry);

        // The user token expiration is much longer than the session expiration.
        // We need to regularly refresh the session to keep the user token alive.
        let mut session_ttl = self.session_ttl();
        debug!(
            "session time to live: {:.0}s",
            session_ttl.as_secs_f32().ceil()
        );
        let session_expiry = tokio::time::sleep(session_ttl);
        tokio::pin!(session_expiry);

        // The JWT
        let mut jwt_ttl = self.jwt_ttl();
        debug!("jwt time to live: {:.0}s", jwt_ttl.as_secs_f32().ceil());
        let jwt_expiry = tokio::time::sleep(jwt_ttl);
        tokio::pin!(jwt_expiry);

        let config = Some(
            WebSocketConfig::default()
                .max_write_buffer_size(Self::MESSAGE_BUFFER_MAX)
                .max_message_size(Some(Self::MESSAGE_SIZE_MAX))
                .max_frame_size(Some(Self::FRAME_SIZE_MAX)),
        );

        let (ws_stream, _) = if let Some(proxy) = proxy::Http::from_env() {
            info!("using proxy: {proxy}");
            let tcp_stream = proxy.connect_async(&uri).await?;
            tokio_tungstenite::client_async_tls_with_config(request, tcp_stream, config, None)
                .await?
        } else {
            tokio_tungstenite::connect_async_with_config(request, config, false).await?
        };

        let (websocket_tx, mut websocket_rx) = ws_stream.split();
        self.websocket_tx = Some(websocket_tx);

        self.subscribe(Ident::Stream).await?;
        self.subscribe(Ident::RemoteDiscover).await?;

        if self.eavesdrop {
            warn!("not discoverable: eavesdropping on websocket");
        } else {
            info!("ready for discovery");
        }

        let loop_result = loop {
            tokio::select! {
                biased;

                () = &mut self.watchdog_tx, if self.is_connected() => {
                    if let Err(e) = self.send_ping().await {
                        error!("error sending ping: {e}");
                    }
                }

                () = &mut self.watchdog_rx, if self.is_connected() => {
                    error!("controller is not responding");
                    let _drop = self.disconnect().await;
                }

                () = &mut token_expiry => {
                    break Err(Error::deadline_exceeded("user token expired"));
                }

                () = &mut session_expiry => {
                    // Soft failure: we will try to con
                    match tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.refresh()).await {
                        Ok(inner) => {
                            match inner {
                                Ok(()) => {
                                    debug!("session renewed");
                                    session_ttl = self.session_ttl();
                                }
                                Err(e) => {
                                    error!("session renewal failed: {e}");
                                }
                            }
                        }
                        Err(e) => error!("session renewal timed out: {e}"),
                    }

                    debug!("session time to live: {:.0}s", session_ttl.as_secs_f32().ceil());
                    if let Some(deadline) = tokio::time::Instant::now().checked_add(session_ttl) {
                        session_expiry.as_mut().reset(deadline);
                    }
                }

                () = &mut jwt_expiry => {
                    // Soft failure: JWT logins are not required to interact with the gateway.
                    match tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.renew_login()).await {
                        Ok(inner) => {
                            match inner {
                                Ok(()) => {
                                    debug!("jwt renewed");
                                    jwt_ttl = self.jwt_ttl();
                                }
                                Err(e) => {
                                    warn!("jwt renewal failed: {e}");
                                }
                            }
                        }
                        Err(e) => warn!("jwt renewal timed out: {e}"),
                    }

                    debug!("jwt time to live: {:.0}s", jwt_ttl.as_secs_f32().ceil());
                    if let Some(deadline) = tokio::time::Instant::now().checked_add(jwt_ttl) {
                        jwt_expiry.as_mut().reset(deadline);
                    }
                }

                Some(token_ttl) = self.time_to_live_rx.recv() => {
                    if let Some(deadline) = tokio::time::Instant::now().checked_add(token_ttl) {
                        token_expiry.as_mut().reset(deadline);
                    }
                }

                () = &mut self.reporting_timer, if self.is_connected() => {
                    if let Err(e) = self.report_playback_progress().await {
                        error!("error reporting playback progress: {e}");
                    }
                }

                Some(message) = websocket_rx.next() => {
                    match message {
                        Ok(message) => {
                            // Do not parse exceedingly large messages to
                            // prevent out of memory conditions.
                            let message_size = message.len();
                            if message_size > Self::MESSAGE_SIZE_MAX {
                                error!("ignoring oversized message with {message_size} bytes");
                                continue;
                            }

                            if let ControlFlow::Break(e) = self.handle_message(&message).await {
                                break Err(Error::internal(format!("error handling message: {e}")));
                            }
                        }

                        Err(e) => break Err(Error::cancelled(e.to_string())),
                    }
                }

                Err(e) = self.player.run(), if self.player.is_started() => break Err(e),

                Some(event) = self.event_rx.recv() => {
                    self.handle_event(event).await;
                }
            }
        };

        self.stop().await;
        loop_result
    }

    /// Processes received events.
    ///
    /// Handles:
    /// * Play - Track started
    /// * Pause - Playback paused
    /// * `TrackChanged` - New track active
    /// * Connected - Controller connected
    /// * Disconnected - Controller disconnected
    ///
    /// Executes hook script if configured.
    ///
    /// # Arguments
    ///
    /// * `event` - Event to process
    #[allow(clippy::too_many_lines)]
    async fn handle_event(&mut self, event: Event) {
        let mut command = self.hook.as_ref().map(Command::new);
        let track_id = self.player.track().map(Track::id);

        debug!("handling event: {event:?}");

        // Report playback progress without waiting for the next reporting interval,
        // so the UI refreshes immediately
        if let Event::Pause | Event::Play = event {
            let _ = self.report_playback_progress().await;
        }

        // Next, execute the rest of the event handling logic
        match event {
            Event::Play => {
                if let Some(track_id) = track_id {
                    // Report the playback stream.
                    if let Err(e) = self.report_playback(track_id).await {
                        error!("error streaming {track_id}: {e}");
                    }

                    if self.is_flow() {
                        // Extend the queue if the player is near the end.
                        if self
                            .queue
                            .as_ref()
                            .map_or(0, |queue| queue.tracks.len())
                            .saturating_sub(self.player.position())
                            <= 2
                        {
                            if let Err(e) = self.extend_queue().await {
                                error!("error extending queue: {e}");
                            }
                        }
                    }

                    if let Some(command) = command.as_mut() {
                        command
                            .env("EVENT", "playing")
                            .env("TRACK_ID", track_id.to_string());
                    }
                }
            }

            Event::Pause => {
                if let Some(command) = command.as_mut() {
                    command.env("EVENT", "paused");
                }
            }

            Event::TrackChanged => {
                if let Some(track) = self.player.track() {
                    if let Some(command) = command.as_mut() {
                        let codec = track.codec().map_or("Unknown".to_string(), |codec| {
                            codec.to_string().to_uppercase()
                        });

                        let bitrate = track.bitrate();
                        let bitrate = match bitrate {
                            Some(bitrate) => {
                                if bitrate >= 1000 {
                                    format!(" {}M", bitrate.to_f32_lossy() / 1000.)
                                } else {
                                    format!(" {bitrate}K")
                                }
                            }
                            // If bitrate is unknown, show codec only.
                            None => String::default(),
                        };

                        let channels =
                            match track.channels.unwrap_or(track.typ().default_channels()) {
                                1 => "Mono".to_string(),
                                2 => "Stereo".to_string(),
                                3 => "2.1 Stereo".to_string(),
                                6 => "5.1 Surround Sound".to_string(),
                                other => format!("{other} channels"),
                            };
                        let decoded = format!(
                            "PCM {} bit {} kHz, {channels}",
                            track.bits_per_sample.unwrap_or(DEFAULT_BITS_PER_SAMPLE),
                            track
                                .sample_rate
                                .unwrap_or(DEFAULT_SAMPLE_RATE)
                                .to_f32_lossy()
                                / 1000.0,
                        );

                        command
                            .env("EVENT", "track_changed")
                            .env("TRACK_TYPE", track.typ().to_string())
                            .env("TRACK_ID", track.id().to_string())
                            .env("ARTIST", track.artist())
                            .env("COVER_ID", track.cover_id())
                            .env("FORMAT", format!("{codec}{bitrate}"))
                            .env("DECODER", decoded);

                        if let Some(title) = track.title() {
                            command.env("TITLE", title);
                        }
                        if let Some(album_title) = track.album_title() {
                            command.env("ALBUM_TITLE", album_title);
                        }
                        if let Some(duration) = track.duration() {
                            command.env("DURATION", duration.as_secs().to_string());
                        }
                    }
                }
            }

            Event::Connected => {
                if let Some(command) = command.as_mut() {
                    command
                        .env("EVENT", "connected")
                        .env("USER_ID", self.user_id().to_string())
                        .env("USER_NAME", self.gateway.user_name().unwrap_or_default());
                }
            }

            Event::Disconnected => {
                if let Some(command) = command.as_mut() {
                    command.env("EVENT", "disconnected");
                }
            }
        }

        if let Some(command) = command.as_mut() {
            if let Err(e) = command.spawn() {
                error!("failed to spawn hook script: {e}");
            }
        }
    }

    /// Returns whether current queue is a Flow (personalized radio).
    ///
    /// Examines queue context to identify Flow queues by checking:
    /// * Queue has contexts
    /// * First context is a user mix
    ///
    /// # Returns
    ///
    /// * `true` - Queue is a Flow queue
    /// * `false` - Queue is not Flow or no queue exists
    #[inline]
    fn is_flow(&self) -> bool {
        self.queue.as_ref().is_some_and(|queue| {
            queue
                .contexts
                .first()
                .unwrap_or_default()
                .container
                .mix
                .typ
                .enum_value_or_default()
                == MixType::MIX_TYPE_USER
        })
    }

    /// Resets the receive watchdog timer.
    ///
    /// Called when messages are received from the controller to prevent connection timeout.
    #[inline]
    fn reset_watchdog_rx(&mut self) {
        if let Some(deadline) = from_now(Self::WATCHDOG_RX_TIMEOUT) {
            self.watchdog_rx.as_mut().reset(deadline);
        }
    }

    /// Resets the transmit watchdog timer.
    ///
    /// Called when messages are sent to the controller to maintain heartbeat timing.
    #[inline]
    fn reset_watchdog_tx(&mut self) {
        if let Some(deadline) = from_now(Self::WATCHDOG_TX_TIMEOUT) {
            self.watchdog_tx.as_mut().reset(deadline);
        }
    }

    /// Resets the playback reporting timer.
    ///
    /// Schedules the next progress report according to the reporting interval.
    #[inline]
    fn reset_reporting_timer(&mut self) {
        if let Some(deadline) = from_now(Self::REPORTING_INTERVAL) {
            self.reporting_timer.as_mut().reset(deadline);
        }
    }

    /// Stops the client and cleans up resources.
    ///
    /// * Disconnects from controller if connected
    /// * Processes remaining events
    /// * Unsubscribes from channels
    pub async fn stop(&mut self) {
        if self.is_connected() {
            if let Err(e) = self.disconnect().await {
                error!("error disconnecting: {e}");
            }
        }

        // Handle any remaining events without closing the event channel,
        // so it will work when the client is restarted.
        while !self.event_rx.is_empty() {
            if let Some(event) = self.event_rx.recv().await {
                self.handle_event(event).await;
            }
        }

        // Cancel any remaining subscriptions not handled by `disconnect`.
        let subscriptions = self.subscriptions.clone();
        for ident in subscriptions {
            if self.unsubscribe(ident).await.is_ok() {
                self.subscriptions.remove(&ident);
            }
        }

        // Soft failure: JWT logins are not required to interact with the gateway.
        match tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.logout()).await {
            Ok(inner) => {
                if let Err(e) = inner {
                    warn!("jwt logout failed: {e}");
                } else {
                    debug!("jwt logged out");
                }
            }
            Err(e) => warn!("jwt logout timed out: {e}"),
        }
    }

    /// Creates a message targeted at a specific device.
    ///
    /// # Arguments
    ///
    /// * `destination` - Target device ID
    /// * `channel` - Message channel
    /// * `body` - Message content
    ///
    /// # Returns
    ///
    /// Formatted message ready for sending.
    fn message(&self, destination: DeviceId, channel: Channel, body: Body) -> Message {
        let contents = Contents {
            ident: channel.ident,
            headers: Headers {
                from: self.device_id.clone(),
                destination: Some(destination),
            },
            body,
        };

        Message::Send { channel, contents }
    }

    /// Creates a command message for a device.
    ///
    /// Convenience wrapper around `message()` for command channel.
    ///
    /// # Arguments
    ///
    /// * `destination` - Target device ID
    /// * `body` - Command content
    fn command(&self, destination: DeviceId, body: Body) -> Message {
        let remote_command = self.channel(Ident::RemoteCommand);
        self.message(destination, remote_command, body)
    }

    /// Creates a discovery message for a device.
    ///
    /// Convenience wrapper around `message()` for discovery channel.
    ///
    /// # Arguments
    ///
    /// * `destination` - Target device ID
    /// * `body` - Discovery content
    fn discover(&self, destination: DeviceId, body: Body) -> Message {
        let remote_discover = self.channel(Ident::RemoteDiscover);
        self.message(destination, remote_discover, body)
    }

    /// Reports track playback to Deezer.
    ///
    /// # Arguments
    ///
    /// * `track_id` - ID of track being played
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active connection
    /// * Message send fails
    async fn report_playback(&mut self, track_id: TrackId) -> Result<()> {
        if let ConnectionState::Connected { session_id, .. } = &self.connection_state {
            let message = Message::StreamSend {
                channel: self.channel(Ident::Stream),
                contents: stream::Contents {
                    action: stream::Action::Play,
                    ident: stream::Ident::Limitation,
                    value: stream::Value {
                        user: self.user_id(),
                        uuid: *session_id,
                        track_id,
                    },
                },
            };

            self.send_message(message).await
        } else {
            Err(Error::failed_precondition(
                "playback reporting should have an active connection".to_string(),
            ))
        }
    }

    /// Disconnects from the current controller.
    ///
    /// Sends a close message to the controller and resets connection state.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Sending a close message fails
    async fn disconnect(&mut self) -> Result<()> {
        self.send_close().await?;
        self.reset_states();
        Ok(())
    }

    /// Handles device discovery request from a controller.
    ///
    /// Creates and caches a connection offer, then sends it to the requesting controller.
    /// Caches the controller's discovery session to prevent duplicate offers showing up
    /// in older Deezer apps.
    ///
    /// # Arguments
    ///
    /// * `from` - ID of requesting controller
    /// * `discovery_session_id` - Unique identifier for this discovery session
    ///
    /// # Implementation Notes
    ///
    /// Controllers send discovery requests approximately every 2 seconds until accepting an offer.
    /// To prevent older Deezer app versions from showing duplicate remote entries, this method:
    ///
    /// 1. Checks if controller already has a cached session
    /// 2. Only generates and sends new offer if controller hasn't been seen
    /// 3. Caches session ID mapped to controller ID
    ///
    /// Caching by device ID rather than session ID is more memory efficient since the same
    /// controllers typically reconnect multiple times with different session IDs.
    ///
    /// # Errors
    ///
    /// Returns error if message send fails
    async fn handle_discovery_request(
        &mut self,
        from: DeviceId,
        discovery_session_id: String,
    ) -> Result<()> {
        if self
            .discovery_sessions
            .get(&from)
            .is_none_or(|session_id| *session_id != discovery_session_id)
        {
            // Controllers keep sending discovery requests about every two seconds
            // until it accepts some offer. Sometimes they take up on old requests,
            // and we don't really care as long as it is directed to us.
            let offer = Body::ConnectionOffer {
                message_id: crate::Uuid::fast_v4().to_string(),
                from: self.device_id.clone(),
                device_name: self.device_name.clone(),
                device_type: self.device_type,
            };

            let discover = self.discover(from.clone(), offer);
            self.send_message(discover).await?;

            // Cache the discovery session ID to prevent multiple offers showing up in the Deezer
            // app. Newer versions of the app will ignore multiple offers from the same remote, but
            // older versions will show the same remote multiple times.
            self.discovery_sessions.insert(from, discovery_session_id);
        }

        Ok(())
    }

    /// Handles connection request from a controller.
    ///
    /// Validates the connection and establishes control session if:
    /// * Client is available for connections
    /// * Required channel subscriptions succeed
    ///
    /// Note: Offer ID is ignored as controllers may use old offers.
    /// What matters is that the request is directed at this device.
    ///
    /// # Arguments
    ///
    /// * `from` - ID of connecting controller
    /// * `offer_id` - ID of previous connection offer (ignored)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Client is not available
    /// * Channel subscription fails
    /// * Message send fails
    async fn handle_connect(&mut self, from: DeviceId, _offer_id: Option<String>) -> Result<()> {
        if self.discovery_state == DiscoveryState::Taken {
            debug!("not allowing interruptions from {from}");

            // This is a known and valid condition. Return `Ok` so the
            // control flow may continue.
            return Ok(());
        }

        // Subscribe to both channels. If one fails, try to roll back.
        self.subscribe(Ident::RemoteQueue).await?;
        if let Err(e) = self.subscribe(Ident::RemoteCommand).await {
            let _drop = self.unsubscribe(Ident::RemoteQueue).await;
            return Err(e);
        }

        let message_id = crate::Uuid::fast_v4().to_string();
        let ready = Body::Ready {
            message_id: message_id.clone(),
        };

        let command = self.command(from.clone(), ready);
        self.send_message(command).await?;

        self.discovery_state = DiscoveryState::Connecting {
            controller: from,
            ready_message_id: message_id,
        };

        Ok(())
    }

    /// Checks if client has active controller connection.
    ///
    /// # Returns
    ///
    /// * true - Connected to controller
    /// * false - Not connected
    #[must_use]
    #[inline]
    fn is_connected(&self) -> bool {
        if let ConnectionState::Connected { .. } = &self.connection_state {
            return true;
        }

        false
    }

    /// Returns ID of currently connected controller if any.
    ///
    /// Checks both active connections and pending connections:
    /// * Returns controller ID from active connection if present
    /// * Returns controller ID from pending connection if no active connection
    /// * Returns None if no controller connection exists
    ///
    /// # Returns
    ///
    /// * `Some(DeviceId)` - ID of connected or connecting controller
    /// * `None` - No controller connection exists
    #[inline]
    fn controller(&self) -> Option<DeviceId> {
        if let ConnectionState::Connected { controller, .. } = &self.connection_state {
            return Some(controller.clone());
        }

        if let DiscoveryState::Connecting { controller, .. } = &self.discovery_state {
            return Some(controller.clone());
        }

        None
    }

    /// Sends a close message to the currently connected controller.
    ///
    /// Sends a close command if there is either:
    /// * An active controller connection
    /// * A pending controller connection
    ///
    /// # Errors
    ///
    /// Returns error if message send fails
    async fn send_close(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            let close = Body::Close {
                message_id: crate::Uuid::fast_v4().to_string(),
            };

            let command = self.command(controller.clone(), close);
            self.send_message(command).await?;
        }

        Ok(())
    }

    /// Handles status message from controller.
    ///
    /// Processes command status and updates connection state.
    /// During connection handshake, establishes full connection and:
    /// * Updates connection state
    /// * Sets discovery state
    /// * Loads user settings
    ///
    /// # Arguments
    ///
    /// * `from` - Controller device ID
    /// * `command_id` - ID of command being acknowledged
    /// * `status` - Command status
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Status indicates command failure
    /// * Connection state transition invalid
    /// * Message send fails
    /// * Volume setting fails
    async fn handle_status(
        &mut self,
        from: DeviceId,
        command_id: &str,
        status: Status,
    ) -> Result<()> {
        if status != Status::OK {
            return Err(Error::failed_precondition(format!(
                "controller failed to process {command_id}"
            )));
        }

        if let DiscoveryState::Connecting {
            controller,
            ready_message_id,
        } = self.discovery_state.clone()
        {
            if from == controller && command_id == ready_message_id {
                if self.is_connected() {
                    self.send_close().await?;
                }

                if self.interruptions {
                    self.discovery_state = DiscoveryState::Available;
                } else {
                    self.discovery_state = DiscoveryState::Taken;
                }

                // The unique session ID is used when reporting playback.
                self.connection_state = ConnectionState::Connected {
                    controller: from,
                    session_id: crate::Uuid::fast_v4().into(),
                };

                info!("connected to {controller}");
                if let Err(e) = self.event_tx.send(Event::Connected) {
                    error!("failed to send connected event: {e}");
                }

                // Refresh user token to reload configuration (normalization, audio quality)
                // If token refresh fails, assume ARL expired and signal client restart
                let (user_token, token_ttl) = match self.user_token().await {
                    Ok((token, ttl)) => (Ok(token), ttl),
                    Err(e) => (Err(e), Duration::ZERO),
                };

                // Sending a zero duration will cause the client to restart.
                if let Err(e) = self.time_to_live_tx.send(token_ttl).await {
                    error!("failed to send user token time to live: {e}");
                }

                self.user_token = Some(user_token?);
                self.set_player_settings();

                return Ok(());
            }

            return Err(Error::failed_precondition(
                "should match controller and ready message".to_string(),
            ));
        }

        // Ignore other status messages.
        Ok(())
    }

    /// Handles close request from controller.
    ///
    /// Cleans up connection state and subscriptions.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller
    /// * Unsubscribe fails
    async fn handle_close(&mut self) -> Result<()> {
        if self.controller().is_some() {
            self.unsubscribe(Ident::RemoteQueue).await?;
            self.unsubscribe(Ident::RemoteCommand).await?;

            self.reset_states();
            return Ok(());
        }

        Err(Error::failed_precondition(
            "close should have an active connection".to_string(),
        ))
    }

    /// Resets connection and discovery states.
    ///
    /// Called when a connection terminates to:
    /// * Clear controller association
    /// * Reset connection state
    /// * Reset discovery state
    /// * Restore initial volume for next connection
    /// * Flush cached tokens
    /// * Emit disconnect event
    ///
    /// The initial volume is reactivated during reset to ensure it will be
    /// applied again when a new controller connects.
    fn reset_states(&mut self) {
        if let Some(controller) = self.controller() {
            info!("disconnected from {controller}");

            if let Err(e) = self.event_tx.send(Event::Disconnected) {
                error!("failed to send disconnected event: {e}");
            }
        }

        // Ensure the player releases the output device.
        self.player.stop();

        // Restore the initial volume for the next connection.
        if let InitialVolume::Inactive(initial_volume) = self.initial_volume {
            self.initial_volume = InitialVolume::Active(initial_volume);
        }

        // Force the user token to be reloaded on the next connection.
        self.gateway.flush_user_token();

        // Reset the connection and discovery states.
        self.connection_state = ConnectionState::Disconnected;
        self.discovery_state = DiscoveryState::Available;
    }

    /// Handles queue publication from controller.
    ///
    /// Updates local queue and configures player:
    /// * Stores queue metadata
    /// * Resolves track information
    /// * Updates player queue
    /// * Handles deferred position
    /// * Extends Flow queues
    ///
    /// # Arguments
    ///
    /// * `list` - Published queue content
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Queue resolution fails
    /// * Flow extension fails
    async fn handle_publish_queue(&mut self, list: queue::List) -> Result<()> {
        let shuffled = if list.shuffled { "(shuffled)" } else { "" };
        info!("setting queue to {} {shuffled}", list.id);

        // Await with timeout in order to prevent blocking the select loop.
        let queue = tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.list_to_queue(&list))
            .await??;

        let tracks: Vec<_> = queue.into_iter().map(Track::from).collect();

        self.queue = Some(list);
        self.player.set_queue(tracks);

        if let Some(position) = self.deferred_position.take() {
            self.set_position(position);
        }

        if self.is_flow() {
            self.extend_queue().await?;
        }

        Ok(())
    }

    /// Sends ping message to controller.
    ///
    /// Part of connection keepalive mechanism.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller
    /// * Message send fails
    async fn send_ping(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            let ping = Body::Ping {
                message_id: crate::Uuid::fast_v4().to_string(),
            };

            let command = self.command(controller.clone(), ping);
            return self.send_message(command).await;
        }

        Err(Error::failed_precondition(
            "ping should have an active connection".to_string(),
        ))
    }

    /// Extends Flow queue and notifies controller.
    ///
    /// Fetches more personalized recommendations when:
    /// * Current queue is Flow
    /// * Near end of current tracks
    ///
    /// Updates both local state and remote controller by:
    /// 1. Fetching new tracks
    /// 2. Updating local queue and player
    /// 3. Publishing updated queue to controller
    /// 4. Requesting controller UI refresh
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active queue exists
    /// * Track fetch fails
    /// * Controller communication fails
    async fn extend_queue(&mut self) -> Result<()> {
        let user_id = self.user_id();

        if let Some(list) = self.queue.as_mut() {
            let new_queue =
                tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.user_radio(user_id))
                    .await??;

            let new_tracks: Vec<_> = new_queue.into_iter().map(Track::from).collect();

            let new_list: Vec<_> = new_tracks
                .iter()
                .map(|track| queue::Track {
                    id: track.id().to_string(),
                    ..Default::default()
                })
                .collect();

            debug!("extending queue with {} tracks", new_tracks.len());

            list.tracks.extend(new_list);
            self.player.extend_queue(new_tracks);
            self.refresh_queue().await
        } else {
            Err(Error::failed_precondition(
                "cannot extend queue: queue is missing",
            ))
        }
    }

    /// Publishes updated queue to controller and requests UI refresh.
    ///
    /// Called after queue modifications to:
    /// 1. Send updated queue state to controller
    /// 2. Request controller to refresh its UI
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller connection exists
    /// * Queue publication fails
    /// * Refresh request fails
    ///
    /// # Notes
    ///
    /// This is typically called after operations that modify the queue like:
    /// * Extending Flow recommendations
    /// * Updating shuffle order
    /// * Changing repeat mode
    async fn refresh_queue(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            // First publish the new queue to the controller.
            if let Some(queue) = self.queue.as_mut() {
                queue.id = crate::Uuid::fast_v4().to_string();
            }
            self.publish_queue().await?;

            // Then signal the controller to refresh its UI.
            let contents = Body::RefreshQueue {
                message_id: crate::Uuid::fast_v4().to_string(),
            };

            let channel = self.channel(Ident::RemoteQueue);
            let refresh_queue = self.message(controller.clone(), channel, contents);
            self.send_message(refresh_queue).await
        } else {
            Err(Error::failed_precondition(
                "refresh should have an active connection".to_string(),
            ))
        }
    }

    /// Handles a refresh queue request from the controller.
    ///
    /// Simply republishes our current queue state in response to
    /// the controller's request for a refresh.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active queue exists
    /// * No active controller connection
    /// * Message send fails
    /// * Progress report fails
    async fn handle_refresh_queue(&mut self) -> Result<()> {
        if let Some(queue) = self.queue.as_mut() {
            queue.id = crate::Uuid::fast_v4().to_string();
            self.publish_queue().await?;
            self.report_playback_progress().await
        } else {
            Err(Error::failed_precondition(
                "queue refresh should have a published queue".to_string(),
            ))
        }
    }

    /// Publishes current queue state to the remote controller.
    ///
    /// Sends a `PublishQueue` message containing:
    /// * New message ID
    /// * Current queue state
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller connection
    /// * No queue exists to publish
    /// * Message send fails
    async fn publish_queue(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            if let Some(queue) = self.queue.as_ref() {
                let contents = Body::PublishQueue {
                    message_id: crate::Uuid::fast_v4().to_string(),
                    queue: queue.clone(),
                };

                let channel = self.channel(Ident::RemoteQueue);
                let publish_queue = self.message(controller.clone(), channel, contents);
                self.send_message(publish_queue).await
            } else {
                Err(Error::failed_precondition(
                    "queue refresh should have a published queue".to_string(),
                ))
            }
        } else {
            Err(Error::failed_precondition(
                "queue refresh should have an active connection".to_string(),
            ))
        }
    }

    /// Sends acknowledgement for a command.
    ///
    /// # Arguments
    ///
    /// * `acknowledgement_id` - ID of command to acknowledge
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller
    /// * Message send fails
    async fn send_acknowledgement(&mut self, acknowledgement_id: &str) -> Result<()> {
        if let Some(controller) = self.controller() {
            let acknowledgement = Body::Acknowledgement {
                message_id: crate::Uuid::fast_v4().to_string(),
                acknowledgement_id: acknowledgement_id.to_string(),
            };

            let command = self.command(controller, acknowledgement);
            return self.send_message(command).await;
        }

        Err(Error::failed_precondition(
            "acknowledgement should have an active connection".to_string(),
        ))
    }

    /// Handles skip command from controller.
    ///
    /// Updates player state according to skip parameters:
    /// * Queue position
    /// * Playback progress
    /// * Playback state
    /// * Shuffle mode
    /// * Repeat mode
    /// * Volume
    ///
    /// Handles state updates gracefully:
    /// 1. Acknowledges command receipt
    /// 2. Updates player state (continuing on errors)
    /// 3. Refreshes queue if shuffle state changed
    /// 4. Reports playback progress
    /// 5. Sends status to controller
    ///
    /// # Arguments
    ///
    /// * `message_id` - Command ID for acknowledgement
    /// * `queue_id` - Target queue identifier
    /// * `item` - Target track and position
    /// * `progress` - Playback progress
    /// * `should_play` - Whether to start playback
    /// * `set_shuffle` - New shuffle mode
    /// * `set_repeat_mode` - New repeat mode
    /// * `set_volume` - New volume level
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller
    /// * Message send fails
    /// * All state updates fail
    #[expect(clippy::too_many_arguments)]
    async fn handle_skip(
        &mut self,
        message_id: &str,
        queue_id: Option<&str>,
        item: Option<QueueItem>,
        progress: Option<Percentage>,
        should_play: Option<bool>,
        set_shuffle: Option<bool>,
        set_repeat_mode: Option<RepeatMode>,
        set_volume: Option<Percentage>,
    ) -> Result<()> {
        // Check for controller, not if we are connected: the first `Skip`
        // message is received during the handshake, before the connection is
        // ready.
        if self.controller().is_some() {
            self.send_acknowledgement(message_id).await?;

            // Remember to refresh the queue if the shuffle mode changes.
            let refresh_queue = self.queue.as_ref().map(|queue| queue.shuffled) != set_shuffle;

            // Attempt to set the player state, including reordering the queue if the shuffle mode
            // has changed. No need to print the error message, as the method will log it.
            let state_set = self
                .set_player_state(
                    queue_id,
                    item,
                    progress,
                    should_play,
                    set_shuffle,
                    set_repeat_mode,
                    set_volume,
                )
                .is_ok();

            // Refresh the queue if the shuffle mode has changed.
            if refresh_queue && self.queue.as_ref().map(|queue| queue.shuffled) == set_shuffle {
                if let Err(e) = self.refresh_queue().await {
                    error!("error refreshing queue: {e}");
                }
            }

            // Report playback progress regardless of the state setting result - it can be that
            // *some* state was set, but not all of it.
            if let Err(e) = self.report_playback_progress().await {
                error!("error reporting playback progress: {e}");
            }

            // The status response to the first skip, that is received during the initial handshake
            // ahead of the queue publication, should be "1" (Error).
            let status = if state_set && self.queue.is_some() {
                Status::OK
            } else {
                Status::Error
            };

            self.send_status(message_id, status).await?;

            Ok(())
        } else {
            Err(Error::failed_precondition(
                "skip should have an active connection".to_string(),
            ))
        }
    }

    /// Sets the current playback position in the queue.
    ///
    /// Handles position conversion for shuffled queues:
    /// * For unshuffled queues - Uses position directly
    /// * For shuffled queues - Maps position through shuffle order
    ///
    /// # Arguments
    ///
    /// * `position` - Target position in the queue (in display order)
    ///
    /// After position calculation, updates the player's actual queue position.
    #[inline]
    fn set_position(&mut self, position: usize) {
        let mut position = position;
        if let Some(queue) = self.queue.as_ref() {
            if queue.shuffled {
                position = queue.tracks_order[position] as usize;
            }
        }

        self.player.set_position(position);
    }

    /// Updates player state based on controller commands.
    ///
    /// Applies changes to:
    /// * Queue position
    /// * Playback progress (ignores for livestreams)
    /// * Playback state (with initial volume application on play)
    /// * Shuffle mode and track order
    /// * Repeat mode
    /// * Volume level (respecting initial volume until client takes control)
    ///
    /// Initial volume is applied when:
    /// * First starting playback
    /// * Initial volume is active
    /// * Client hasn't taken control
    ///
    /// Handles error cases gracefully:
    /// * Progress setting failures
    /// * Volume setting failures
    /// * Playback state failures
    ///
    /// Returns the first error encountered but continues attempting remaining updates.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - Target queue identifier
    /// * `item` - Target track and position
    /// * `progress` - Playback progress
    /// * `should_play` - Whether to start playback
    /// * `set_shuffle` - New shuffle mode
    /// * `set_repeat_mode` - New repeat mode
    /// * `set_volume` - New volume level
    ///
    /// # Errors
    ///
    /// Returns error if any update fails, but attempts all updates regardless
    #[expect(clippy::too_many_arguments)]
    pub fn set_player_state(
        &mut self,
        queue_id: Option<&str>,
        item: Option<QueueItem>,
        progress: Option<Percentage>,
        should_play: Option<bool>,
        set_shuffle: Option<bool>,
        set_repeat_mode: Option<RepeatMode>,
        set_volume: Option<Percentage>,
    ) -> Result<()> {
        let mut result = Ok(());

        let current = self.player.position();
        let mut target = current;
        if let Some(item) = item {
            target = item.position;

            // Sometimes Deezer sends a skip message ahead of a queue publication.
            // In this case, we defer setting the position until the queue is published.
            if self
                .queue
                .as_ref()
                .is_some_and(|local| queue_id.is_some_and(|remote| local.id == remote))
            {
                self.set_position(target);
            } else {
                self.deferred_position = Some(target);
            }
        }

        // Only set the track position if it's for the current track. If it's for another track,
        // then we just skipped to it and we need the player to let it start playing from the
        // beginning.
        if target == current {
            if let Some(progress) = progress {
                if self
                    .player
                    .track()
                    .is_some_and(super::track::Track::is_livestream)
                {
                    trace!("ignoring set_progress for livestream");
                } else if let Err(e) = self.player.set_progress(progress) {
                    error!("error setting playback position: {e}");
                    result = Err(e);
                }
            }
        }

        if let Some(shuffle) = set_shuffle {
            if self
                .queue
                .as_ref()
                .is_some_and(|queue| queue.shuffled != shuffle)
            {
                if shuffle {
                    self.shuffle_queue(ShuffleAction::Shuffle);
                } else {
                    self.shuffle_queue(ShuffleAction::Unshuffle);
                }

                if let Some(queue) = self.queue.as_mut() {
                    let reordered_queue: Vec<_> = queue
                        .tracks
                        .iter()
                        .filter_map(|track| track.id.parse().ok())
                        .collect();
                    self.player.reorder_queue(&reordered_queue);
                }
            }
        }

        if let Some(repeat_mode) = set_repeat_mode {
            self.player.set_repeat_mode(repeat_mode);
        }

        if let Some(mut volume) = set_volume {
            if let InitialVolume::Active(initial_volume) = self.initial_volume {
                if volume < Percentage::ONE_HUNDRED {
                    // If the volume is set to a value less than 1.0, we stop using the initial
                    // volume.
                    self.initial_volume = InitialVolume::Inactive(initial_volume);
                } else {
                    volume = initial_volume;
                }
            }

            if let Err(e) = self.player.set_volume(volume) {
                error!("error setting volume: {e}");
                result = Err(e);
            }
        }

        if let Some(should_play) = should_play {
            if should_play {
                // Open the output device ourselves so we can set the initial volume
                // before starting playback.
                match self.player.start() {
                    Ok(()) => {
                        if let InitialVolume::Active(initial_volume) = self.initial_volume {
                            if let Err(e) = self.player.set_volume(initial_volume) {
                                error!("error setting initial volume: {e}");
                                result = Err(e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("error opening output device: {e}");
                        result = Err(e);
                    }
                }
            }

            if let Err(e) = self.player.set_playing(should_play) {
                error!("error setting playback state: {e}");
                result = Err(e);
            }
        }

        result
    }

    /// Shuffles or unshuffles the current queue.
    ///
    /// # Arguments
    ///
    /// * `action` - Whether to shuffle or unshuffle the queue
    ///
    /// When shuffling:
    /// * Randomizes track order
    /// * Stores original order for unshuffling
    /// * Updates shuffle state
    ///
    /// When unshuffling:
    /// * Restores original track order
    /// * Clears stored order
    /// * Updates shuffle state
    ///
    /// No effect if no queue exists.
    #[expect(clippy::cast_possible_truncation)]
    fn shuffle_queue(&mut self, action: ShuffleAction) {
        if let Some(queue) = self.queue.as_mut() {
            match action {
                ShuffleAction::Shuffle => {
                    info!("shuffling queue");

                    let len = queue.tracks.len();
                    let mut order: Vec<usize> = (0..len).collect();
                    fastrand::shuffle(&mut order);

                    let mut tracks = Vec::with_capacity(len);
                    for i in &order {
                        tracks.push(queue.tracks[*i].clone());
                    }

                    queue.tracks = tracks;
                    queue.tracks_order = order.iter().map(|position| *position as u32).collect();
                    queue.shuffled = true;
                }

                ShuffleAction::Unshuffle => {
                    info!("unshuffling queue");

                    let len = queue.tracks.len();
                    let mut tracks = Vec::with_capacity(len);
                    for i in 0..len {
                        if let Some(position) = queue
                            .tracks_order
                            .iter()
                            .position(|position| *position == i as u32)
                        {
                            tracks.push(queue.tracks[position].clone());
                        }
                    }

                    queue.tracks = tracks;
                    queue.tracks_order = Vec::new();
                    queue.shuffled = false;
                }
            }
        }
    }

    /// Sends command status to controller.
    ///
    /// # Arguments
    ///
    /// * `command_id` - ID of command being acknowledged
    /// * `status` - Command completion status
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller
    /// * Message send fails
    async fn send_status(&mut self, command_id: &str, status: Status) -> Result<()> {
        if let Some(controller) = self.controller() {
            let status = Body::Status {
                message_id: crate::Uuid::fast_v4().to_string(),
                command_id: command_id.to_string(),
                status,
            };

            let command = self.command(controller.clone(), status);
            return self.send_message(command).await;
        }

        Err(Error::failed_precondition(
            "status should have an active connection".to_string(),
        ))
    }

    /// Reports current playback state to controller.
    ///
    /// Sends current:
    /// * Track information
    /// * Playback progress
    /// * Buffer status
    /// * Volume level
    /// * Playback state
    /// * Shuffle/repeat modes
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No active controller
    /// * No active queue
    /// * No current track
    /// * Message send fails
    #[expect(clippy::cast_possible_truncation)]
    async fn report_playback_progress(&mut self) -> Result<()> {
        // Reset the timer regardless of success or failure, to prevent getting
        // stuck in a reporting state.
        self.reset_reporting_timer();

        // TODO : replace `if let Some(x) = y` with `let x = y.ok_or(z)?`
        if let Some(controller) = self.controller() {
            if let Some(track) = self.player.track() {
                let queue = self
                    .queue
                    .as_ref()
                    .ok_or_else(|| Error::internal("no active queue"))?;

                let player_position = self.player.position();
                let mut position = player_position;
                let progress = self.player.progress();

                // If current progress is 100% and there is a track upcoming, then skip this
                // reporting cycle: the next track will be reported very soon instead. This
                // prevents some UI glitches.
                if self.player.is_playing()
                    && progress.is_some_and(|progress| progress >= Percentage::ONE_HUNDRED)
                    && self.player.next_track().is_some()
                {
                    return Ok(());
                }

                // If in shuffle mode, find the position of the current track in the shuffled order.
                if queue.shuffled {
                    position = queue
                        .tracks_order
                        .iter()
                        .position(|i| *i == player_position as u32)
                        .unwrap_or_default();
                }

                let item = QueueItem {
                    queue_id: queue.id.to_string(),
                    track_id: track.id(),
                    position,
                };

                let progress = Body::PlaybackProgress {
                    message_id: crate::Uuid::fast_v4().to_string(),
                    track: item,
                    quality: track.quality(),
                    duration: self.player.duration(),
                    buffered: track.buffered(),
                    volume: self.player.volume(),
                    is_playing: self.player.is_playing(),
                    is_shuffle: queue.shuffled,
                    repeat_mode: self.player.repeat_mode(),
                    progress,
                };

                let command = self.command(controller.clone(), progress);
                self.send_message(command).await?;
            }

            Ok(())
        } else {
            Err(Error::failed_precondition(
                "playback progress should have an active connection".to_string(),
            ))
        }
    }

    /// Handles incoming websocket messages.
    ///
    /// Processes:
    /// * Text messages (JSON protocol messages)
    /// * Ping frames (RFC 6455 compliance)
    /// * Close frames (connection termination)
    ///
    /// # Arguments
    ///
    /// * `message` - Incoming websocket message
    ///
    /// # Returns
    ///
    /// * Continue - Message handled successfully
    /// * Break(Error) - Fatal error occurred
    async fn handle_message(&mut self, message: &WebsocketMessage) -> ControlFlow<Error, ()> {
        match message {
            WebsocketMessage::Text(message) => {
                match serde_json::from_str::<Message>(message.as_str()) {
                    Ok(message) => {
                        match message.clone() {
                            Message::Receive { contents, .. } => {
                                let from = contents.headers.from;

                                // Ignore echoes of own messages.
                                if from == self.device_id {
                                    return ControlFlow::Continue(());
                                }

                                let for_another = contents
                                    .headers
                                    .destination
                                    .is_some_and(|destination| destination != self.device_id);

                                // Only log messages intended for this device or eavesdropping.
                                if !for_another || self.eavesdrop {
                                    if log_enabled!(Level::Trace) {
                                        trace!("{message:#?}");
                                    } else {
                                        debug!("{message}");
                                    }
                                }

                                // Ignore messages not intended for this device.
                                if for_another || self.eavesdrop {
                                    return ControlFlow::Continue(());
                                }

                                if self
                                    .controller()
                                    .is_some_and(|controller| controller == from)
                                {
                                    self.reset_watchdog_rx();
                                }

                                if let Err(e) = self.dispatch(from, contents.body).await {
                                    error!("error handling message: {e}");
                                }
                            }

                            Message::StreamReceive { .. } => {
                                if self.eavesdrop {
                                    if log_enabled!(Level::Trace) {
                                        trace!("{message:#?}");
                                    } else {
                                        debug!("{message}");
                                    }
                                }
                                return ControlFlow::Continue(());
                            }

                            _ => {
                                trace!("ignoring unexpected message: {message:#?}");
                            }
                        }
                    }

                    Err(e) => {
                        error!("error parsing message: {e}");
                        debug!("{message:#?}");
                    }
                }
            }

            // Deezer Connect sends pings as text message payloads, but so far
            // not as websocket frames. Aim for RFC 6455 compliance anyway.
            WebsocketMessage::Ping(payload) => {
                debug!("ping -> pong");
                let pong = Frame::pong(payload.clone());
                if let Err(e) = self.send_frame(WebsocketMessage::Frame(pong)).await {
                    error!("{e}");
                }
            }

            WebsocketMessage::Close(payload) => {
                return ControlFlow::Break(Error::aborted(format!(
                    "connection closed by server: {payload:?}"
                )));
            }

            _ => {
                trace!("ignoring unimplemented frame: {message:#?}");
            }
        }

        ControlFlow::Continue(())
    }

    /// Dispatches protocol messages to appropriate handlers.
    ///
    /// Routes messages based on body type:
    /// * Acknowledgement - Command completion
    /// * Close - Connection termination
    /// * Connect - Connection establishment
    /// * Discovery - Device discovery
    /// * Ping - Connection keepalive
    /// * Queue - Content management
    /// * Skip - Playback control
    /// * Status - Command status
    /// * Stop - Playback control
    ///
    /// # Arguments
    ///
    /// * `from` - Source device ID
    /// * `body` - Message content
    ///
    /// # Errors
    ///
    /// Returns error if message handler fails
    async fn dispatch(&mut self, from: DeviceId, body: Body) -> Result<()> {
        match body {
            // TODO - Think about maintaining a queue of message IDs to be
            // acknowledged, evictingt them one by one.
            Body::Acknowledgement { .. } => Ok(()),

            Body::Close { .. } => self.handle_close().await,

            Body::Connect { from, offer_id, .. } => self.handle_connect(from, offer_id).await,

            Body::DiscoveryRequest {
                from,
                discovery_session,
                ..
            } => self.handle_discovery_request(from, discovery_session).await,

            // Pings don't use dedicated WebSocket frames, but are sent as
            // normal data. An acknowledgement serves as pong.
            Body::Ping { message_id } => self.send_acknowledgement(&message_id).await,

            Body::PublishQueue { queue, .. } => self.handle_publish_queue(queue).await,

            Body::RefreshQueue { .. } => self.handle_refresh_queue().await,

            Body::Skip {
                message_id,
                queue_id,
                track,
                progress,
                should_play,
                set_shuffle,
                set_repeat_mode,
                set_volume,
            } => {
                self.handle_skip(
                    &message_id,
                    queue_id.as_deref(),
                    track,
                    progress,
                    should_play,
                    set_shuffle,
                    set_repeat_mode,
                    set_volume,
                )
                .await
            }

            Body::Status {
                command_id, status, ..
            } => self.handle_status(from, &command_id, status).await,

            Body::Stop { .. } => {
                self.player.pause();
                Ok(())
            }

            Body::ConnectionOffer { .. } | Body::PlaybackProgress { .. } | Body::Ready { .. } => {
                trace!("ignoring message intended for a controller");
                Ok(())
            }
        }
    }

    /// Sends a websocket frame.
    ///
    /// # Arguments
    ///
    /// * `frame` - Frame to send
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No websocket connection
    /// * Send operation fails
    async fn send_frame(&mut self, frame: WebsocketMessage) -> Result<()> {
        match &mut self.websocket_tx {
            Some(tx) => tx.send(frame).await.map_err(Into::into),
            None => Err(Error::unavailable(
                "websocket stream unavailable".to_string(),
            )),
        }
    }

    /// Sends a protocol message.
    ///
    /// Serializes message to JSON and sends as text frame.
    /// Resets watchdog timer on success.
    ///
    /// # Arguments
    ///
    /// * `message` - Protocol message to send
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * JSON serialization fails
    /// * Frame send fails
    async fn send_message(&mut self, message: Message) -> Result<()> {
        // Reset the timer regardless of success or failure, to prevent getting
        // stuck in a reporting state.
        self.reset_watchdog_tx();

        if log_enabled!(Level::Trace) {
            trace!("{message:#?}");
        } else {
            debug!("{message}");
        }

        let json = serde_json::to_string(&message)?;
        let frame = WebsocketMessage::Text(json.into());
        self.send_frame(frame).await
    }

    /// Subscribes to a protocol channel.
    ///
    /// Only subscribes if not already subscribed.
    ///
    /// # Arguments
    ///
    /// * `ident` - Channel identifier
    ///
    /// # Errors
    ///
    /// Returns error if subscription message fails
    async fn subscribe(&mut self, ident: Ident) -> Result<()> {
        if !self.subscriptions.contains(&ident) {
            let channel = self.channel(ident);

            let subscribe = Message::Subscribe { channel };
            self.send_message(subscribe).await?;

            self.subscriptions.insert(ident);
        }

        Ok(())
    }

    /// Unsubscribes from a protocol channel.
    ///
    /// Only unsubscribes if currently subscribed.
    ///
    /// # Arguments
    ///
    /// * `ident` - Channel identifier
    ///
    /// # Errors
    ///
    /// Returns error if unsubscribe message fails
    async fn unsubscribe(&mut self, ident: Ident) -> Result<()> {
        if self.subscriptions.contains(&ident) {
            let channel = self.channel(ident);

            let unsubscribe = Message::Unsubscribe { channel };
            self.send_message(unsubscribe).await?;

            self.subscriptions.remove(&ident);
        }

        Ok(())
    }

    /// Returns current user ID.
    ///
    /// Returns unspecified ID if no user token available.
    #[must_use]
    #[inline]
    fn user_id(&self) -> UserId {
        self.user_token
            .as_ref()
            .map_or(UserId::Unspecified, |token| token.user_id)
    }

    /// Creates channel descriptor for given identifier.
    ///
    /// Sets source and destination based on:
    /// * Channel type
    /// * Current user ID
    ///
    /// # Arguments
    ///
    /// * `ident` - Channel identifier
    ///
    /// # Returns
    ///
    /// Channel descriptor for protocol messages
    #[must_use]
    #[inline]
    fn channel(&self, ident: Ident) -> Channel {
        let user_id = self.user_id();
        let from = if let Ident::UserFeed(_) = ident {
            UserId::Unspecified
        } else {
            user_id
        };

        Channel {
            from,
            to: user_id,
            ident,
        }
    }
}
