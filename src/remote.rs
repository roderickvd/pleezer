//! Remote control protocol implementation for Deezer Connect.
//!
//! This module implements the client side of the Deezer Connect protocol,
//! enabling remote control functionality between devices. It handles:
//! * Device discovery and connection
//! * Authentication and session management
//! * Command processing
//! * Queue synchronization and manipulation (shuffle, repeat)
//! * Playback reporting
//! * Event notifications
//!
//! # Protocol Flow
//!
//! 1. Connection Establishment
//!    * Client connects to Deezer websocket
//!    * Authenticates with user token
//!    * Subscribes to required channels
//!
//! 2. Device Discovery
//!    * Client announces availability
//!    * Controllers send discovery requests
//!    * Client responds with connection offers
//!
//! 3. Control Session
//!    * Controller initiates connection
//!    * Client accepts if available
//!    * Establishes session and channel subscriptions
//!    * Commands flow between devices
//!    * Queue and playback state synchronized (including shuffle)
//!
//! # Connection States
//!
//! A client progresses through several states:
//! * Disconnected - Initial state
//! * Available - Ready for discovery
//! * Connecting - Accepting controller
//! * Connected - Active control session
//! * Taken - Connection locked (if interruptions disabled)
//!
//! # Message Types
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

use std::{collections::HashSet, ops::ControlFlow, pin::Pin, process::Command, time::Duration};

use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use log::Level;
use semver;
use tokio_tungstenite::{
    tungstenite::{
        client::ClientRequestBuilder, protocol::frame::Frame, Message as WebsocketMessage,
    },
    MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

use crate::{
    arl::Arl,
    config::{Config, Credentials},
    error::{Error, Result},
    events::Event,
    gateway::Gateway,
    player::Player,
    protocol::connect::{
        queue::{self, ContainerType, MixType},
        stream, Body, Channel, Contents, DeviceId, DeviceType, Headers, Ident, Message, Percentage,
        QueueItem, RepeatMode, Status, UserId,
    },
    proxy,
    tokens::UserToken,
    track::{Track, TrackId},
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
/// * Active - Set volume and remain active until client sets below maximum
/// * Inactive - Initial volume has been superseded by client control
/// * Disabled - No initial volume configured
#[derive(Copy, Clone, Debug, PartialEq)]
enum InitialVolume {
    /// Initial volume is active and will be applied
    Active(Percentage),
    /// Initial volume is stored but inactive
    Inactive(Percentage),
    /// No initial volume configured
    Disabled,
}

/// Calculates a future time instant by adding seconds to now.
///
/// # Arguments
///
/// * `seconds` - Duration to add to current time
///
/// # Returns
///
/// * Some(Instant) - Future time if addition succeeds
/// * None - If addition would overflow
#[must_use]
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

    /// Maximum allowed websocket message size in bytes.
    const MESSAGE_SIZE_MAX: usize = 8192;

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

            initial_volume,
            interruptions: config.interruptions,
            hook: config.hook.clone(),

            queue: None,
            deferred_position: None,

            eavesdrop: config.eavesdrop,
        })
    }

    /// Attempts to login using email and password credentials.
    ///
    /// # Arguments
    ///
    /// * `email` - User's email address
    /// * `password` - User's password
    ///
    /// # Returns
    ///
    /// An ARL token for future authentication.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Login credentials are invalid
    /// * Network request fails
    /// * Gateway response is invalid
    async fn login(&mut self, email: &str, password: &str) -> Result<Arl> {
        let arl = self.gateway.login(email, password).await?;

        // Use `arl:?` to print as `Debug`, which is redacted.
        trace!("arl: {arl:?}");

        Ok(arl)
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
            let token = self.gateway.user_token().await?;

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

    /// Starts the client and handles control messages.
    ///
    /// Establishes websocket connection, authenticates, and begins processing:
    /// * Controller discovery
    /// * Command messages
    /// * Playback state updates
    /// * Connection maintenance
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Authentication fails
    /// * Websocket connection fails
    /// * Message handling fails critically
    pub async fn start(&mut self) -> Result<()> {
        if let Credentials::Login { email, password } = &self.credentials.clone() {
            info!("logging in with email and password");
            // We can drop the result because the ARL is stored as a cookie.
            let _arl = self.login(email, password).await?;
        } else {
            info!("using ARL from secrets file");
        }

        let (user_token, time_to_live) = self.user_token().await?;
        debug!("user id: {}", user_token.user_id);

        // Set timer for user token expiration. Wake a short while before
        // actual expiration. This prevents API request errors when the
        // expiration is checked with only a few seconds on the clock.
        let expiry = tokio::time::sleep(time_to_live);
        tokio::pin!(expiry);

        let uri = format!(
            "wss://live.deezer.com/ws/{}?version={}",
            user_token, self.version
        );
        let mut request = ClientRequestBuilder::new(uri.parse::<http::Uri>()?);

        self.user_token = Some(user_token);

        // Decorate the websocket request with the same cookies as the gateway.
        if let Some(cookies) = self.gateway.cookies() {
            if let Ok(cookie_str) = cookies.to_str() {
                request = request.with_header("Cookie", cookie_str);
            } else {
                warn!("unable to set cookie header on websocket");
            }
        }

        let (ws_stream, _) = if let Some(proxy) = proxy::Http::from_env() {
            info!("using proxy: {proxy}");
            let tcp_stream = proxy.connect_async(&uri).await?;
            tokio_tungstenite::client_async_tls(request, tcp_stream).await?
        } else {
            tokio_tungstenite::connect_async(request).await?
        };

        let (websocket_tx, mut websocket_rx) = ws_stream.split();
        self.websocket_tx = Some(websocket_tx);

        self.subscribe(Ident::Stream).await?;
        self.subscribe(Ident::RemoteDiscover).await?;

        // Register playback event handler.
        self.player.register(self.event_tx.clone());

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

                () = &mut expiry => {
                    break Err(Error::deadline_exceeded("user token expired"));
                }

                Some(time_to_live) = self.time_to_live_rx.recv() => {
                    if let Some(deadline) = tokio::time::Instant::now().checked_add(time_to_live) {
                        expiry.as_mut().reset(deadline);
                    }
                }

                () = &mut self.reporting_timer, if self.is_connected() && self.player.is_playing() => {
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

                            match self.handle_message(&message).await {
                                ControlFlow::Continue(()) => continue,

                                ControlFlow::Break(e) => {
                                    break Err(Error::internal(format!("error handling message: {e}")));
                                }
                            }
                        }
                        Err(e) => error!("error receiving message: {e}"),
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
    async fn handle_event(&mut self, event: Event) {
        let mut command = self.hook.as_ref().map(Command::new);
        let track_id = self.player.track().map(Track::id);

        debug!("handling event: {event:?}");

        match event {
            Event::Play => {
                if let Some(track_id) = track_id {
                    // Report playback progress without waiting for the next
                    // reporting interval, so the UI refreshes immediately.
                    let _ = self.report_playback_progress().await;

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
                            .env("TRACK_ID", shell_escape(&track_id.to_string()));
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
                        command
                            .env("EVENT", "track_changed")
                            .env("TRACK_ID", shell_escape(&track.id().to_string()))
                            .env("TITLE", shell_escape(track.title()))
                            .env("ARTIST", shell_escape(track.artist()))
                            .env("ALBUM_TITLE", shell_escape(track.album_title()))
                            .env("ALBUM_COVER", shell_escape(track.album_cover()))
                            .env(
                                "DURATION",
                                shell_escape(&track.duration().as_secs().to_string()),
                            );
                    }
                }
            }

            Event::Connected => {
                if let Some(command) = command.as_mut() {
                    command
                        .env("EVENT", "connected")
                        .env("USER_ID", shell_escape(&self.user_id().to_string()))
                        .env(
                            "USER_NAME",
                            shell_escape(self.gateway.user_name().unwrap_or_default()),
                        );
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

    /// Checks whether the current queue is a Flow (personalized radio) queue.
    ///
    /// Examines the queue context to determine if it represents a personalized radio stream.
    ///
    /// # Returns
    ///
    /// * `true` - Current queue is a Flow queue
    /// * `false` - Current queue is not Flow or no queue exists
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
    fn reset_watchdog_rx(&mut self) {
        if let Some(deadline) = from_now(Self::WATCHDOG_RX_TIMEOUT) {
            self.watchdog_rx.as_mut().reset(deadline);
        }
    }

    /// Resets the transmit watchdog timer.
    ///
    /// Called when messages are sent to the controller to maintain heartbeat timing.
    fn reset_watchdog_tx(&mut self) {
        if let Some(deadline) = from_now(Self::WATCHDOG_TX_TIMEOUT) {
            self.watchdog_tx.as_mut().reset(deadline);
        }
    }

    /// Resets the playback reporting timer.
    ///
    /// Schedules the next progress report according to the reporting interval.
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
    /// Creates and caches a connection offer, then sends it to the
    /// requesting controller.
    ///
    /// # Arguments
    ///
    /// * `from` - ID of requesting controller
    ///
    /// # Errors
    ///
    /// Returns error if message send fails.
    async fn handle_discovery_request(&mut self, from: DeviceId) -> Result<()> {
        // Controllers keep sending discovery requests about every two seconds
        // until it accepts some offer. Sometimes they take up on old requests,
        // and we don't really care as long as it is directed to us.
        let offer = Body::ConnectionOffer {
            message_id: crate::Uuid::fast_v4().to_string(),
            from: self.device_id.clone(),
            device_name: self.device_name.clone(),
            device_type: self.device_type,
        };

        let discover = self.discover(from, offer);
        self.send_message(discover).await
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
    fn is_connected(&self) -> bool {
        if let ConnectionState::Connected { .. } = &self.connection_state {
            return true;
        }

        false
    }

    /// Returns ID of currently connected controller if any.
    ///
    /// # Returns
    ///
    /// * Some(DeviceId) - ID of connected controller
    /// * None - No controller connected
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
    /// * Starts playback device
    /// * Applies initial volume if configured
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

                // Refresh the user token on every reconnection in order to reload the user
                // configuration, like normalization and audio quality. If this fails, then:
                // - assume that the arl expired
                // - return a deadline exceeded error
                // - so that the client can be stopped (and restarted)
                let result = tokio::time::timeout(Self::NETWORK_TIMEOUT, self.user_token()).await?;
                let time_to_live = result.as_ref().map_or(Duration::ZERO, |result| result.1);
                if let Err(e) = self.time_to_live_tx.send(time_to_live).await {
                    error!("failed to send user token time to live: {e}");
                }
                self.user_token = Some(result?.0);

                self.set_player_settings();
                self.player.start()?;

                if let InitialVolume::Active(initial_volume) = self.initial_volume {
                    debug!("initial volume: {initial_volume}");
                    self.player.set_volume(initial_volume)?;
                }

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
    /// * Restore initial volume activation
    /// * Flush cached tokens
    /// * Emit disconnect event
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
        let container_type = list
            .contexts
            .first()
            .unwrap_or_default()
            .container
            .typ
            .enum_value_or_default();

        let shuffled = if list.shuffled { "(shuffled)" } else { "" };
        info!("setting queue to {} {shuffled}", list.id);

        // Await with timeout in order to prevent blocking the select loop.
        let queue = match container_type {
            ContainerType::CONTAINER_TYPE_LIVE => {
                error!("live radio is not supported yet");
                Vec::new()
            }
            ContainerType::CONTAINER_TYPE_PODCAST => {
                error!("podcasts are not supported yet");
                Vec::new()
            }
            _ => {
                tokio::time::timeout(Self::NETWORK_TIMEOUT, self.gateway.list_to_queue(&list))
                    .await??
            }
        };

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
    /// * Player state update fails
    /// * Message send fails
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

            self.set_player_state(
                queue_id,
                item,
                progress,
                should_play,
                set_shuffle,
                set_repeat_mode,
                set_volume,
            )
            .await?;

            // The status response to the first skip, that is received during the initial handshake
            // ahead of the queue publication, should be "1" (Error).
            let status = if self.player.track().is_some() {
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
    /// * Playback progress
    /// * Playback state
    /// * Shuffle mode and track order
    /// * Repeat mode
    /// * Volume level (respecting initial volume until client takes control)
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
    /// Returns error if:
    /// * Progress setting fails
    /// * Progress reporting fails
    #[expect(clippy::too_many_arguments)]
    pub async fn set_player_state(
        &mut self,
        queue_id: Option<&str>,
        item: Option<QueueItem>,
        progress: Option<Percentage>,
        should_play: Option<bool>,
        set_shuffle: Option<bool>,
        set_repeat_mode: Option<RepeatMode>,
        set_volume: Option<Percentage>,
    ) -> Result<()> {
        if let Some(item) = item {
            let position = item.position;

            // Sometimes Deezer sends a skip message ahead of a queue publication.
            // In this case, we defer setting the position until the queue is published.
            if self
                .queue
                .as_ref()
                .is_some_and(|local| queue_id.is_some_and(|remote| local.id == remote))
            {
                self.set_position(position);
            } else {
                self.deferred_position = Some(position);
            }
        }

        if let Some(progress) = progress {
            self.player.set_progress(progress)?;
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
                    self.refresh_queue().await?;
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
            }
        }

        if let Some(should_play) = should_play {
            if let Err(e) = self.player.set_playing(should_play) {
                error!("error setting playback state: {e}");
            }
        }

        // TODO: move to caller so we also report on failure
        self.report_playback_progress().await
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
                    queue.tracks_order.clear();
                    queue.tracks_order.shrink_to_fit();
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
                    .ok_or(Error::internal("no active queue"))?;

                let player_position = self.player.position();
                let mut position = player_position;
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
                    duration: track.duration(),
                    buffered: track.buffered(),
                    progress: self.player.progress(),
                    volume: self.player.volume(),
                    is_playing: self.player.is_playing(),
                    is_shuffle: queue.shuffled,
                    repeat_mode: self.player.repeat_mode(),
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
                match serde_json::from_str::<Message>(message) {
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
                )))
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

            Body::DiscoveryRequest { from, .. } => self.handle_discovery_request(from).await,

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

            Body::Stop { .. } => self.player.pause(),

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
        let frame = WebsocketMessage::Text(json);
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

/// Escapes a string for use in shell commands.
///
/// Wraps shell-escape crate's functionality for string escaping.
///
/// # Arguments
///
/// * `s` - String to escape
///
/// # Returns
///
/// Shell-safe escaped string
fn shell_escape(s: &str) -> String {
    shell_escape::escape(s.into()).to_string()
}
