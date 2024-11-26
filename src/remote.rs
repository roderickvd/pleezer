use std::{collections::HashSet, ops::ControlFlow, pin::Pin, process::Command, time::Duration};

use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use log::Level;
use lru_time_cache::LruCache;
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
    error::{Error, ErrorKind, Result},
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
    device_id: DeviceId,
    device_name: String,
    device_type: DeviceType,

    credentials: Credentials,
    gateway: Gateway,

    user_token: Option<UserToken>,
    time_to_live_tx: tokio::sync::mpsc::Sender<Duration>,
    time_to_live_rx: tokio::sync::mpsc::Receiver<Duration>,

    version: String,
    websocket_tx:
        Option<SplitSink<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, WebsocketMessage>>,

    subscriptions: HashSet<Ident>,

    connection_state: ConnectionState,
    watchdog_rx: Pin<Box<tokio::time::Sleep>>,
    watchdog_tx: Pin<Box<tokio::time::Sleep>>,

    discovery_state: DiscoveryState,
    connection_offers: LruCache<String, DeviceId>,

    event_rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    event_tx: tokio::sync::mpsc::UnboundedSender<Event>,

    interruptions: bool,
    hook: Option<String>,

    player: Player,
    reporting_timer: Pin<Box<tokio::time::Sleep>>,

    queue: Option<queue::List>,
    deferred_position: Option<usize>,

    eavesdrop: bool,
}

#[derive(Clone, Debug, PartialOrd, Ord, PartialEq, Eq)]
enum DiscoveryState {
    Available,
    Connecting {
        controller: DeviceId,
        ready_message_id: String,
    },
    Taken,
}

#[derive(Clone, Debug, PartialEq)]
enum ConnectionState {
    Disconnected,
    Connected {
        controller: DeviceId,
        session_id: Uuid,
    },
}

#[must_use]
fn from_now(seconds: Duration) -> Option<tokio::time::Instant> {
    tokio::time::Instant::now().checked_add(seconds)
}

impl Client {
    const NETWORK_TIMEOUT: Duration = Duration::from_secs(1);
    const TOKEN_EXPIRATION_THRESHOLD: Duration = Duration::from_secs(60);
    const REPORTING_INTERVAL: Duration = Duration::from_secs(3);
    const WATCHDOG_RX_TIMEOUT: Duration = Duration::from_secs(10);
    const WATCHDOG_TX_TIMEOUT: Duration = Duration::from_secs(5);

    const MESSAGE_SIZE_MAX: usize = 8192;

    /// TODO
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the `app_version` in `config` is not in [`SemVer`] format
    ///
    /// [SemVer]: https://semver.org/
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

        // Controllers send discovery requests every two seconds.
        let time_to_live = Duration::from_secs(5);
        let connection_offers = LruCache::with_expiry_duration(time_to_live);

        // Timers are set in the message handlers. They should be moved into
        // a state variant once `select!` supports `if let` statements:
        // https://github.com/tokio-rs/tokio/issues/4173
        let reporting_timer = tokio::time::sleep(Duration::ZERO);
        let watchdog_rx = tokio::time::sleep(Duration::ZERO);
        let watchdog_tx = tokio::time::sleep(Duration::ZERO);

        let (time_to_live_tx, time_to_live_rx) = tokio::sync::mpsc::channel(1);
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();

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
            connection_offers,

            interruptions: config.interruptions,
            hook: config.hook.clone(),

            queue: None,
            deferred_position: None,

            eavesdrop: config.eavesdrop,
        })
    }

    async fn login(&mut self, email: &str, password: &str) -> Result<Arl> {
        let arl = self.gateway.login(email, password).await?;

        // Use `arl:?` to print as `Debug`, which is redacted.
        trace!("arl: {arl:?}");

        Ok(arl)
    }

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

    fn set_player_settings(&mut self) {
        let audio_quality = self.gateway.audio_quality().unwrap_or_default();
        info!("user casting quality: {audio_quality}");
        self.player.set_audio_quality(audio_quality);

        let normalization = self.gateway.normalization();
        let gain_target_db = self.gateway.target_gain();
        info!("volume normalization to {gain_target_db} dB: {normalization}");
        self.player.set_gain_target_db(gain_target_db);
        self.player.set_normalization(normalization);

        if let Some(license_token) = self.gateway.license_token() {
            self.player.set_license_token(license_token);
        }

        self.player.set_media_url(self.gateway.media_url());
    }

    /// TODO
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the websocket could not be connected to
    /// - sending or receiving messages failed
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
                                    if e.kind == ErrorKind::DeadlineExceeded {
                                        info!("stopping client: {}", e.to_string());
                                        self.gateway.flush_user_token();
                                        break Ok(());
                                    }

                                    break Err(Error::internal(format!("error handling message: {e}")));
                                }
                            }
                        }
                        Err(e) => error!("error receiving message: {e}"),
                    }
                }

                Err(e) = self.player.run() => break Err(e),

                Some(event) = self.event_rx.recv() => {
                    self.handle_event(event).await;
                }
            }
        };

        self.stop().await;

        loop_result
    }

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

            // Generate a new list ID for the UI to pick up.
            list.id = Uuid::new_v4().to_string();

            debug!(
                "extending queue {} with {} tracks",
                list.id,
                new_tracks.len()
            );

            list.tracks.extend(new_list);
            self.player.extend_queue(new_tracks);

            // Refresh the controller's queue with the tracks that we got from the user radio.
            self.handle_refresh_queue().await
        } else {
            Err(Error::failed_precondition(
                "cannot extend queue: queue is missing",
            ))
        }
    }

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

    fn reset_watchdog_rx(&mut self) {
        if let Some(deadline) = from_now(Self::WATCHDOG_RX_TIMEOUT) {
            self.watchdog_rx.as_mut().reset(deadline);
        }
    }

    fn reset_watchdog_tx(&mut self) {
        if let Some(deadline) = from_now(Self::WATCHDOG_TX_TIMEOUT) {
            self.watchdog_tx.as_mut().reset(deadline);
        }
    }

    fn reset_reporting_timer(&mut self) {
        if let Some(deadline) = from_now(Self::REPORTING_INTERVAL) {
            self.reporting_timer.as_mut().reset(deadline);
        }
    }

    pub async fn stop(&mut self) {
        let _drop = self.disconnect().await;

        // Cancel any remaining subscriptions not handled by `disconnect`.
        let subscriptions = self.subscriptions.clone();
        for ident in subscriptions {
            if self.unsubscribe(ident).await.is_ok() {
                self.subscriptions.remove(&ident);
            }
        }
    }

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

    fn command(&self, destination: DeviceId, body: Body) -> Message {
        let remote_command = self.channel(Ident::RemoteCommand);
        self.message(destination, remote_command, body)
    }

    fn discover(&self, destination: DeviceId, body: Body) -> Message {
        let remote_discover = self.channel(Ident::RemoteDiscover);
        self.message(destination, remote_discover, body)
    }

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

            self.send_message(message).await?;
        }

        Err(Error::failed_precondition(
            "playback reporting should have an active connection".to_string(),
        ))
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            let close = Body::Close {
                message_id: Uuid::new_v4().into(),
            };

            let command = self.command(controller.clone(), close);
            self.send_message(command).await?;

            self.reset_states();
            return Ok(());
        }

        Err(Error::failed_precondition(
            "disconnect should have an active connection".to_string(),
        ))
    }

    async fn handle_discovery_request(&mut self, from: DeviceId) -> Result<()> {
        // Controllers keep sending discovery requests about every two seconds
        // until it accepts some offer. `connection_offers` implements a LRU
        // cache to evict stale offers.
        let message_id = Uuid::new_v4().to_string();
        self.connection_offers
            .insert(message_id.clone(), from.clone());

        let offer = Body::ConnectionOffer {
            message_id,
            from: self.device_id.clone(),
            device_name: self.device_name.clone(),
            device_type: self.device_type,
        };

        let discover = self.discover(from, offer);
        self.send_message(discover).await
    }

    async fn handle_connect(&mut self, from: DeviceId, offer_id: Option<String>) -> Result<()> {
        let controller = offer_id
            .and_then(|offer_id| self.connection_offers.remove(&offer_id))
            .ok_or_else(|| Error::failed_precondition("connection offer should be active"))?;

        if controller != from {
            return Err(Error::failed_precondition(format!(
                "connection offer for {controller} should be for {from}"
            )));
        }

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

        let message_id = Uuid::new_v4().to_string();
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

    #[must_use]
    fn is_connected(&self) -> bool {
        if let ConnectionState::Connected { .. } = &self.connection_state {
            return true;
        }

        false
    }

    fn controller(&self) -> Option<DeviceId> {
        if let ConnectionState::Connected { controller, .. } = &self.connection_state {
            return Some(controller.clone());
        }

        if let DiscoveryState::Connecting { controller, .. } = &self.discovery_state {
            return Some(controller.clone());
        }

        None
    }

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
                if let ConnectionState::Connected { controller, .. } = &self.connection_state {
                    // Evict the active connection.
                    let close = Body::Close {
                        message_id: Uuid::new_v4().into(),
                    };

                    let command = self.command(controller.clone(), close);
                    self.send_message(command).await?;
                }

                if self.interruptions {
                    self.discovery_state = DiscoveryState::Available;
                } else {
                    self.discovery_state = DiscoveryState::Taken;
                }

                // Refreshed the user token on every reconnection in order to reload the user
                // configuration, like normalization and audio quality.
                let (user_token, time_to_live) = self.user_token().await?;
                self.user_token = Some(user_token);
                self.set_player_settings();

                // Inform the select loop about the new time to live.
                if let Err(e) = self.time_to_live_tx.send(time_to_live).await {
                    error!("failed to send user token time to live: {e}");
                }

                // The unique session ID is used when reporting playback.
                self.connection_state = ConnectionState::Connected {
                    controller: from,
                    session_id: Uuid::new_v4(),
                };

                info!("connected to {controller}");

                if let Err(e) = self.event_tx.send(Event::Connected) {
                    error!("failed to send connected event: {e}");
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

    fn reset_states(&mut self) {
        if let Some(controller) = self.controller() {
            info!("disconnected from {controller}");

            if let Err(e) = self.event_tx.send(Event::Disconnected) {
                error!("failed to send disconnected event: {e}");
            }
        }

        // Force the user token to be reloaded on the next connection.
        self.gateway.flush_user_token();

        self.connection_state = ConnectionState::Disconnected;
        self.discovery_state = DiscoveryState::Available;
    }

    async fn handle_publish_queue(&mut self, list: queue::List) -> Result<()> {
        // TODO : does it really matter whether there's an active connection?
        if self.controller().is_some() {
            let container_type = list
                .contexts
                .first()
                .unwrap_or_default()
                .container
                .typ
                .enum_value_or_default();

            info!("setting queue to {}", list.id);

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
                self.player.set_position(position);
            }

            if self.is_flow() {
                self.extend_queue().await?;
            }

            return Ok(());
        }

        Err(Error::failed_precondition(
            "queue publication should have an active connection".to_string(),
        ))
    }

    async fn send_ping(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            let ping = Body::Ping {
                message_id: Uuid::new_v4().into(),
            };

            let command = self.command(controller.clone(), ping);
            return self.send_message(command).await;
        }

        Err(Error::failed_precondition(
            "ping should have an active connection".to_string(),
        ))
    }

    async fn handle_refresh_queue(&mut self) -> Result<()> {
        if let Some(controller) = self.controller() {
            if let Some(ref queue) = self.queue {
                // First publish the queue to the controller.
                let contents = Body::PublishQueue {
                    message_id: Uuid::new_v4().into(),
                    queue: queue.clone(),
                };

                let channel = self.channel(Ident::RemoteQueue);
                let publish_queue = self.message(controller.clone(), channel, contents);
                self.send_message(publish_queue).await?;

                // Then signal the controller to refresh its UI.
                let contents = Body::RefreshQueue {
                    message_id: Uuid::new_v4().into(),
                };

                let refresh_queue =
                    self.message(self.controller().unwrap().clone(), channel, contents);
                self.send_message(refresh_queue).await
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

    async fn send_acknowledgement(&mut self, acknowledgement_id: &str) -> Result<()> {
        if let Some(controller) = self.controller() {
            let acknowledgement = Body::Acknowledgement {
                message_id: Uuid::new_v4().into(),
                acknowledgement_id: acknowledgement_id.to_string(),
            };

            let command = self.command(controller, acknowledgement);
            return self.send_message(command).await;
        }

        Err(Error::failed_precondition(
            "acknowledgement should have an active connection".to_string(),
        ))
    }

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

    /// # Errors
    ///
    /// Will return `Err` if:
    /// - `progress` could not be set
    /// - playback progress could not be reported
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
                self.player.set_position(position);
            } else {
                self.deferred_position = Some(position);
            }
        }

        if let Some(progress) = progress {
            self.player.set_progress(progress)?;
        }

        if let Some(shuffle) = set_shuffle {
            self.player.set_shuffle(shuffle);
        }

        if let Some(repeat_mode) = set_repeat_mode {
            self.player.set_repeat_mode(repeat_mode);
        }

        if let Some(volume) = set_volume {
            self.player.set_volume(volume);
        }

        if let Some(should_play) = should_play {
            self.player.set_playing(should_play);
        }

        self.report_playback_progress().await
    }

    async fn send_status(&mut self, command_id: &str, status: Status) -> Result<()> {
        if let Some(controller) = self.controller() {
            let status = Body::Status {
                message_id: Uuid::new_v4().into(),
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

                let item = QueueItem {
                    queue_id: queue.id.to_string(),
                    track_id: track.id(),
                    position: self.player.position(),
                };

                let progress = Body::PlaybackProgress {
                    message_id: Uuid::new_v4().into(),
                    track: item,
                    quality: track.quality(),
                    duration: track.duration(),
                    buffered: track.buffered(),
                    progress: self.player.progress(),
                    volume: self.player.volume(),
                    is_playing: self.player.is_playing(),
                    is_shuffle: self.player.shuffle(),
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

            Body::ConnectionOffer { .. }
            | Body::PlaybackProgress { .. }
            | Body::Ready { .. }
            | Body::RefreshQueue { .. } => {
                trace!("ignoring message intended for a controller");
                Ok(())
            }
        }
    }

    async fn send_frame(&mut self, frame: WebsocketMessage) -> Result<()> {
        match &mut self.websocket_tx {
            Some(tx) => tx.send(frame).await.map_err(Into::into),
            None => Err(Error::unavailable(
                "websocket stream unavailable".to_string(),
            )),
        }
    }

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

    async fn subscribe(&mut self, ident: Ident) -> Result<()> {
        if !self.subscriptions.contains(&ident) {
            let channel = self.channel(ident);

            let subscribe = Message::Subscribe { channel };
            self.send_message(subscribe).await?;

            self.subscriptions.insert(ident);
        }

        Ok(())
    }

    async fn unsubscribe(&mut self, ident: Ident) -> Result<()> {
        if self.subscriptions.contains(&ident) {
            let channel = self.channel(ident);

            let unsubscribe = Message::Unsubscribe { channel };
            self.send_message(unsubscribe).await?;

            self.subscriptions.remove(&ident);
        }

        Ok(())
    }

    #[must_use]
    fn user_id(&self) -> UserId {
        self.user_token
            .as_ref()
            .map_or(UserId::Unspecified, |token| UserId::Id(token.user_id))
    }

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

fn shell_escape(s: &str) -> String {
    shell_escape::escape(s.into()).to_string()
}
