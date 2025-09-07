//! Gateway API client for Deezer services.
//!
//! This module provides access to Deezer's gateway API, handling:
//! * Authentication and session management
//!   - Email/password login
//!   - ARL token authentication
//!   - Optional JWT-based token renewal
//!   - Browser-style session persistence
//! * User data retrieval
//! * Media streaming configuration
//! * Queue and track information
//! * Flow recommendations
//!
//! # Authentication Flow
//!
//! The gateway supports multiple authentication methods:
//!
//! 1. Initial Authentication
//!    * Email/password login (preferred)
//!      - Provides OAuth access token
//!      - Converts to ARL token
//!      - Enables session persistence
//!    * Direct ARL token
//!      - Manual token management
//!      - Requires periodic renewal
//!
//! 2. Session Management
//!    * Browser-style session cookies
//!    * Optional JWT authentication
//!    * Persistent login across restarts
//!    * Automatic session renewal
//!
//! # Media Formats
//!
//! Different content types support different formats:
//! * Songs, podcasts, and livestreams
//! * AAC, FLAC, MP3, and WAV
//! * ADTS and MP4
//!
//! # Example
//!
//! # Example
//!
//! ```rust
//! use pleezer::gateway::Gateway;
//!
//! let mut gateway = Gateway::new(&config)?;
//!
//! // Login with credentials (preferred)
//! let arl = gateway.oauth("user@example.com", "password").await?;
//! gateway.login_with_arl(&arl).await?;
//!
//! // Make authenticated requests
//! let songs = gateway.list_to_queue(&track_list).await?;
//! let recommendations = gateway.user_radio(user_id).await?;
//! let user_data = gateway.refresh().await?;
//! ```

use std::time::SystemTime;

use cookie_store::RawCookie;
use futures_util::TryFutureExt;
use md5::{Digest, Md5};
use reqwest::{
    self,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::Deserialize;
use url::Url;

use crate::{
    arl::Arl,
    config::{Config, Credentials},
    error::{Error, ErrorKind, Result},
    http::Client as HttpClient,
    protocol::{
        self, Codec, auth,
        connect::{
            AudioQuality, UserId,
            queue::{self},
        },
        gateway::{
            self, MediaUrl, Queue, Response, UserData,
            list_data::{
                ListData,
                episodes::{self, EpisodeData},
                livestream::{self, LivestreamData},
                songs::{self, SongData},
            },
            user_radio::{self, UserRadio},
        },
    },
    tokens::UserToken,
};

/// Gateway client for Deezer API access.
///
/// Handles authentication, session management, and API requests to
/// Deezer's gateway endpoints. Maintains user data and authentication
/// state for continuous operation.
pub struct Gateway {
    /// HTTP client with cookie management.
    http_client: HttpClient,

    /// Cached user data from last refresh.
    ///
    /// Contains authentication tokens, preferences, and capabilities.
    // TODO : we probably don't need to retain all user data, all the time
    //       keep what we need here in the gateway, and send the rest off into
    //       a token object
    user_data: Option<UserData>,

    /// Client identifier for API requests.
    client_id: usize,
}

impl Gateway {
    /// Cookie origin URL for Deezer services.
    const COOKIE_ORIGIN: &'static str = "https://deezer.com";

    /// Cookie domain for authentication.
    const COOKIE_DOMAIN: &'static str = ".deezer.com";

    /// Language preference cookie name.
    const LANG_COOKIE: &'static str = "dz_lang";

    /// ARL authentication cookie name.
    const ARL_COOKIE: &'static str = "arl";

    /// JWT authentication service URL
    const JWT_AUTH_URL: &'static str = "https://auth.deezer.com";

    /// JWT login endpoint for ARL authentication
    const JWT_ENDPOINT_LOGIN: &'static str = "/login/arl";

    /// JWT endpoint for renewing authentication
    const JWT_ENDPOINT_RENEW: &'static str = "/login/renew";

    /// JWT endpoint for logging out
    const JWT_ENDPOINT_LOGOUT: &'static str = "/logout";

    /// Gateway API endpoint URL.
    ///
    /// Base URL for all gateway API requests.
    const GATEWAY_URL: &'static str = "https://www.deezer.com/ajax/gw-light.php";

    /// Gateway API version string.
    ///
    /// Protocol version identifier included in all requests.
    /// Matches the version supported by official Deezer clients.
    const GATEWAY_VERSION: &'static str = "1.0";

    /// Gateway API input type identifier.
    ///
    /// Input type code that identifies the request format.
    /// Type 3 represents the standard gateway request format.
    const GATEWAY_INPUT: usize = 3;

    /// OAuth client ID for authentication.
    ///
    /// Application identifier used during OAuth authentication flow.
    /// This will show as a "Hisense TV - V2" Deezer app.
    const OAUTH_CLIENT_ID: usize = 447_462;

    /// OAuth password hashing salt.
    ///
    /// Salt value used in password hash calculation during login.
    /// Combined with client ID and user credentials for secure authentication.
    const OAUTH_SALT: &'static str = "a83bf7f38ad2f137e444727cfc3775cf";

    /// OAuth session ID endpoint.
    ///
    /// URL for initiating OAuth authentication flow.
    /// Used to obtain a session ID before login.
    const OAUTH_SID_URL: &'static str = "https://connect.deezer.com/oauth/auth.php";

    /// OAuth login endpoint.
    ///
    /// URL for performing OAuth login with credentials.
    /// Returns access token on successful authentication.
    const OAUTH_LOGIN_URL: &'static str = "https://connect.deezer.com/oauth/user_auth.php";

    /// Default empty JSON body for requests.
    ///
    /// Used when a request requires a body but has no parameters.
    /// Prevents having to create empty JSON objects repeatedly.
    const EMPTY_JSON_OBJECT: &'static str = "{}";

    /// Returns the cookie origin URL for Deezer services.
    ///
    /// # Panics
    ///
    /// Panics if the hardcoded URL is invalid, which should never happen
    /// as it's a compile-time constant.
    ///
    /// # Internal Use
    ///
    /// This method is used by cookie management functions to ensure
    /// all cookies are properly scoped to the Deezer domain.
    #[must_use]
    fn cookie_origin() -> reqwest::Url {
        reqwest::Url::parse(Self::COOKIE_ORIGIN).expect("invalid cookie origin")
    }

    /// Creates a cookie jar with authentication and language cookies.
    ///
    /// Sets up cookies required for Deezer API access:
    /// * Language preference cookie
    /// * ARL authentication cookie (if using ARL credentials)
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration containing credentials and language settings
    ///
    /// # Cookie Format
    ///
    /// Cookies are set with:
    /// * Domain: deezer.com
    /// * Path: /
    /// * Secure flag
    /// * `HttpOnly` flag
    fn cookie_jar(config: &Config) -> Result<reqwest_cookie_store::CookieStore> {
        let mut cookie_jar = reqwest_cookie_store::CookieStore::new();
        let cookie_origin = Self::cookie_origin();

        let lang_cookie = RawCookie::build((Self::LANG_COOKIE, &config.app_lang))
            .domain(Self::COOKIE_DOMAIN)
            .path("/")
            .secure(true)
            .http_only(true)
            .build();
        if let Err(e) = cookie_jar.insert_raw(&lang_cookie, &cookie_origin) {
            // Log the error but continue, as the language cookie is optional.
            error!("unable to insert language cookie: {e}");
        }

        if let Credentials::Arl(ref arl) = config.credentials {
            let arl_cookie = RawCookie::build((Self::ARL_COOKIE, arl.as_str()))
                .domain(Self::COOKIE_DOMAIN)
                .path("/")
                .secure(true)
                .http_only(true)
                .build();
            if let Err(e) = cookie_jar.insert_raw(&arl_cookie, &cookie_origin) {
                return Err(crate::error::Error::invalid_argument(format!(
                    "failed to insert ARL cookie: {e}"
                )));
            }
        }

        Ok(cookie_jar)
    }

    /// Creates a new gateway client instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration including credentials and client settings
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * User-Agent header cannot be created from config
    /// * OS information cannot be detected
    /// * Cookie creation fails
    pub fn new(config: &Config) -> Result<Self> {
        // Create a new cookie jar and put the cookies in.
        let cookie_jar = Self::cookie_jar(config)?;
        let http_client = HttpClient::with_cookies(config, cookie_jar)?;

        Ok(Self {
            client_id: config.client_id,
            http_client,
            user_data: None,
        })
    }

    /// Returns the current cookie header value, if available.
    ///
    /// Used for authentication in requests to Deezer services.
    ///
    /// # Panics
    ///
    /// Panics if the cookie store mutex is poisoned.
    #[must_use]
    pub fn cookies(&self) -> Option<reqwest_cookie_store::CookieStore> {
        self.http_client
            .cookie_jar
            .as_ref()
            .map(|jar| jar.lock().expect("cookie mutex was poisoned").clone())
    }

    /// Refreshes user data and authentication state.
    ///
    /// Should be called when:
    /// * Starting a new session
    /// * After token expiration
    /// * When user data needs updating
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * ARL token is invalid or expired
    /// * Remote control is disabled
    /// * Too many devices are registered
    /// * Network request fails
    pub async fn refresh(&mut self) -> Result<()> {
        // Send an empty JSON map
        match self
            .request::<UserData>(Self::EMPTY_JSON_OBJECT, None)
            .await
        {
            Ok(response) => {
                if let Some(data) = response.first() {
                    if data.gatekeeps.remote_control.is_some_and(|remote| !remote) {
                        return Err(Error::permission_denied(
                            "remote control is disabled for this account; upgrade your Deezer subscription",
                        ));
                    }
                    if data.user.options.too_many_devices {
                        return Err(Error::resource_exhausted(
                            "too many devices; remove one or more in your account settings",
                        ));
                    }
                    if data.user.options.ads_audio {
                        return Err(Error::unimplemented(
                            "ads are not implemented; upgrade your Deezer subscription",
                        ));
                    }

                    self.set_user_data(data.clone());
                } else {
                    return Err(Error::not_found("no user data received".to_string()));
                }

                Ok(())
            }

            Err(e) => {
                if e.kind == ErrorKind::InvalidArgument {
                    // For an invalid or expired `arl`, the response has some
                    // fields as integer `0` which are normally typed as string,
                    // which causes JSON deserialization to fail.
                    return Err(Error::permission_denied(
                        "arl invalid or expired".to_string(),
                    ));
                }

                Err(e)
            }
        }
    }

    /// Sends a request to the Deezer gateway API.
    ///
    /// Handles:
    /// * API token inclusion
    /// * Request formatting
    /// * Response parsing
    /// * Error mapping
    ///
    /// # Type Parameters
    ///
    /// * `T` - Response type that implements `Method` and `Deserialize`
    ///
    /// # Arguments
    ///
    /// * `body` - Request body content
    /// * `headers` - Optional additional headers
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * URL construction fails
    /// * Network request fails
    /// * HTTP status code is not successful (not 2xx)
    /// * Response isn't valid JSON
    /// * Response can't be parsed as type T
    pub async fn request<T>(
        &mut self,
        body: impl Into<reqwest::Body>,
        headers: Option<HeaderMap>,
    ) -> Result<Response<T>>
    where
        T: std::fmt::Debug + gateway::Method + for<'de> Deserialize<'de>,
    {
        // Get the API token from the user data or use an empty string.
        let api_token = self
            .user_data
            .as_ref()
            .map(|data| data.api_token.as_str())
            .unwrap_or_default();

        // Check the URL early to not needlessly hit the rate limiter.
        let url_str = format!(
            "{}?method={}&input={}&api_version={}&api_token={api_token}&cid={}",
            Self::GATEWAY_URL,
            T::METHOD,
            Self::GATEWAY_INPUT,
            Self::GATEWAY_VERSION,
            self.client_id,
        );
        let url = url_str.parse::<reqwest::Url>()?;

        // Although the bodies of all gateway requests are JSON, the
        // `Content-Type` is not.
        let mut request = self.http_client.text(url, body);
        if let Some(headers) = headers {
            // Add any headers that were passed in.
            request.headers_mut().extend(headers);
        }

        let response = self.http_client.execute(request).await?;
        let body = response.text().await?;
        protocol::json(&body, T::METHOD)
    }

    /// Returns the current license token if available.
    ///
    /// The license token is required for media access.
    #[must_use]
    #[inline]
    pub fn license_token(&self) -> Option<&str> {
        self.user_data
            .as_ref()
            .map(|data| data.user.options.license_token.as_str())
    }

    /// Checks if the current session has expired.
    ///
    /// Returns `true` if:
    /// * No user data is available
    /// * Current time is past expiration time
    #[must_use]
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.expires_at() <= SystemTime::now()
    }

    /// Returns when the current session will expire.
    ///
    /// Returns UNIX epoch if no session is active.
    #[must_use]
    #[inline]
    pub fn expires_at(&self) -> SystemTime {
        if let Some(data) = &self.user_data {
            return data.user.options.expiration_timestamp;
        }

        SystemTime::UNIX_EPOCH
    }

    /// Updates the cached user data.
    #[inline]
    pub fn set_user_data(&mut self, data: UserData) {
        self.user_data = Some(data);
    }

    /// Returns a reference to the current user data if available.
    #[must_use]
    #[inline]
    pub fn user_data(&self) -> Option<&UserData> {
        self.user_data.as_ref()
    }

    /// Returns the user's preferred streaming quality for connected devices.
    ///
    /// Quality settings affect format selection:
    /// * Basic/Standard/High: MP3 at different bitrates (64/128/320 kbps)
    /// * Lossless: FLAC (when available)
    ///
    /// Note: Quality setting only affects songs from Deezer's catalogue.
    /// Other content types (podcasts, livestreams) use their own format selection.
    ///
    /// Returns the default quality if no preference is set.
    #[must_use]
    pub fn audio_quality(&self) -> AudioQuality {
        self.user_data
            .as_ref()
            .map_or(AudioQuality::default(), |data| {
                data.user.audio_settings.connected_device_streaming_preset
            })
    }

    /// Returns the target gain for volume normalization.
    ///
    /// The value is clamped to i8 range as the API might return
    /// out-of-bounds values.
    #[must_use]
    #[expect(clippy::cast_possible_truncation)]
    pub fn target_gain(&self) -> i8 {
        self.user_data
            .as_ref()
            .map(|data| data.gain)
            .unwrap_or_default()
            .target
            .clamp(i64::from(i8::MIN), i64::from(i8::MAX)) as i8
    }

    /// Returns the user's display name if available.
    #[must_use]
    #[inline]
    pub fn user_name(&self) -> Option<&str> {
        self.user_data.as_ref().map(|data| data.user.name.as_str())
    }

    /// Returns the URL for media content requests.
    ///
    /// Returns the default URL if no custom URL is set.
    #[must_use]
    pub fn media_url(&self) -> Url {
        self.user_data
            .as_ref()
            .map_or(MediaUrl::default(), |data| data.media_url.clone())
            .into()
    }

    /// Converts a protocol buffer track list into a queue.
    ///
    /// Fetches detailed track information for each track in the list.
    /// Different track types support different formats:
    /// * Songs: MP3 (CBR) or FLAC
    /// * Episodes: MP3, AAC (ADTS), MP4, or WAV
    /// * Livestreams: AAC (ADTS) or MP3
    /// * Chapters: Not currently supported
    ///
    /// # Arguments
    ///
    /// * `list` - Protocol buffer track list to convert
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Track IDs are invalid
    /// * Track type is unsupported (e.g., audiobooks)
    /// * Network request fails
    /// * Response parsing fails
    pub async fn list_to_queue(&mut self, list: &queue::List) -> Result<Queue> {
        let ids = list
            .tracks
            .iter()
            .map(|track| track.id.parse().map_err(Error::from))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if let Some(first) = list.tracks.first() {
            let response: Response<ListData> = match first.typ.enum_value_or_default() {
                queue::TrackType::TRACK_TYPE_SONG => {
                    let songs = songs::Request { song_ids: ids };
                    let request = serde_json::to_string(&songs)?;
                    self.request::<SongData>(request, None)
                        .map_ok(Into::into)
                        .await?
                }
                queue::TrackType::TRACK_TYPE_EPISODE => {
                    let episodes = episodes::Request { episode_ids: ids };
                    let request = serde_json::to_string(&episodes)?;
                    self.request::<EpisodeData>(request, None)
                        .map_ok(Into::into)
                        .await?
                }
                queue::TrackType::TRACK_TYPE_LIVE => {
                    let radio = livestream::Request {
                        livestream_id: first.id.parse()?,
                        supported_codecs: vec![Codec::ADTS, Codec::MP3],
                    };
                    let request = serde_json::to_string(&radio)?;
                    self.request::<LivestreamData>(request, None)
                        .map_ok(Into::into)
                        .await?
                }
                queue::TrackType::TRACK_TYPE_CHAPTER => {
                    return Err(Error::unimplemented(
                        "audio books not implemented - report what you were trying to play to the developers",
                    ));
                }
            };

            Ok(response.all().clone())
        } else {
            Ok(Queue::default())
        }
    }

    /// Fetches Flow recommendations for a user.
    ///
    /// Flow is Deezer's personalized radio feature.
    ///
    /// # Arguments
    ///
    /// * `user_id` - ID of user to get recommendations for
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Network request fails
    /// * Response parsing fails
    pub async fn user_radio(&mut self, user_id: UserId) -> Result<Queue> {
        let request = user_radio::Request { user_id };
        let body = serde_json::to_string(&request)?;
        match self.request::<UserRadio>(body, None).await {
            Ok(response) => {
                // Transform the `UserRadio` response into a `Queue`. This is done to have
                // `UserRadio` re-use the `ListData` struct (for which `Queue` is an alias).
                Ok(response
                    .all()
                    .clone()
                    .into_iter()
                    .map(|item| item.0)
                    .collect())
            }
            Err(e) => Err(e),
        }
    }

    /// Retrieves an ARL token using an OAuth access token.
    ///
    /// # Arguments
    ///
    /// * `access_token` - OAuth access token from login
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Network request fails
    /// * Response parsing fails
    /// * ARL parsing fails
    /// * No ARL is returned
    pub async fn get_arl(&mut self, access_token: &str) -> Result<Arl> {
        let mut headers = HeaderMap::new();
        headers.try_insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {access_token}"))?,
        )?;

        let arl = self
            .request::<gateway::Arl>(Self::EMPTY_JSON_OBJECT, Some(headers))
            .await
            .and_then(|response| {
                response
                    .first()
                    .map(|result| result.0.clone())
                    .ok_or_else(|| Error::not_found("no arl received".to_string()))
            })?;

        arl.parse::<Arl>()
    }

    /// Returns the user token for remote control functionality.
    ///
    /// Refreshes the session if expired.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Session refresh fails
    /// * User data isn't available
    /// * Remote control is disabled
    /// * Too many devices are registered
    pub async fn user_token(&mut self) -> Result<UserToken> {
        if self.is_expired() {
            debug!("refreshing user token");
            self.refresh().await?;
        }

        match &self.user_data {
            Some(data) => Ok(UserToken {
                user_id: data.user.id,
                token: data.user_token.clone(),
                expires_at: self.expires_at(),
            }),
            None => Err(Error::unavailable("user data unavailable".to_string())),
        }
    }

    /// Invalidates the current user token.
    ///
    /// Forces a refresh on next token request while preserving
    /// other API functionality.
    #[inline]
    pub fn flush_user_token(&mut self) {
        // Force refreshing user data, but do not set `user_data` to `None` so
        // so we can continue using the `api_token` it contains.
        if let Some(data) = self.user_data.as_mut() {
            data.user.options.expiration_timestamp = SystemTime::UNIX_EPOCH;
        }
    }

    /// Logs in with email and password to obtain an ARL token.
    ///
    /// # Arguments
    ///
    /// * `email` - User's email address
    /// * `password` - User's password
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Credentials are invalid
    /// * Email/password length is invalid
    /// * Network request fails
    /// * Response parsing fails
    /// * ARL parsing fails
    pub async fn oauth(&mut self, email: &str, password: &str) -> Result<Arl> {
        // Check email and password length to prevent out-of-memory conditions.
        const LENGTH_CHECK: std::ops::Range<usize> = 1..255;
        if !LENGTH_CHECK.contains(&email.len()) || !LENGTH_CHECK.contains(&password.len()) {
            return Err(Error::out_of_range(
                "email and password must be between 1 and 255 characters".to_string(),
            ));
        }

        // Hash the passwords.
        let password = Md5::digest(password);
        let hash = Md5::digest(format!(
            "{}{email}{password:x}{}",
            Self::OAUTH_CLIENT_ID,
            Self::OAUTH_SALT,
        ));

        // First get a session ID. The response can be ignored because the
        // session ID is stored in the cookie store.
        let request = self.http_client.get(Url::parse(Self::OAUTH_SID_URL)?, "");
        self.http_client.execute(request).await?;

        // Then login and get an access token.
        let query = Url::parse_with_params(
            Self::OAUTH_LOGIN_URL,
            &[
                ("app_id", Self::OAUTH_CLIENT_ID.to_string()),
                ("login", email.to_string()),
                ("password", format!("{password:x}")),
                ("hash", format!("{hash:x}")),
            ],
        )?;

        let request = self.http_client.get(query.clone(), "");
        let response = self.http_client.execute(request).await?;
        let body = response.text().await?;
        let result: auth::User = protocol::json(&body, query.path())
            .map_err(|_| Error::permission_denied("email or password incorrect"))?;

        // Finally use the access token to get an ARL.
        self.get_arl(&result.access_token).await
    }

    /// Authenticates using JWT and ARL token.
    ///
    /// Establishes a persistent session using:
    /// * ARL token for authentication
    /// * Account ID for identification
    /// * JWT for session management
    /// * Cookie-based token storage
    ///
    /// # Arguments
    ///
    /// * `arl` - ARL token for authentication
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * ARL token is invalid
    /// * Network request fails
    /// * Authentication fails
    pub async fn login_with_arl(&mut self, arl: &Arl) -> Result<()> {
        // `c` for cookie (headers), `p` for payload (body)
        let query = Url::parse_with_params(
            &format!("{}{}", Self::JWT_AUTH_URL, Self::JWT_ENDPOINT_LOGIN),
            &[("jo", "p"), ("rto", "c"), ("i", "p")],
        )?;

        let auth = auth::Jwt {
            arl: arl.to_string(),
            account_id: self.user_token().await?.user_id.to_string(),
        };

        let request = self.http_client.json(query, serde_json::to_string(&auth)?);
        self.http_client.execute(request).await?;

        // When successful, the `refresh-token` cookie is set within the HTTP client's cookie store.
        Ok(())
    }

    /// Renews the current login session.
    ///
    /// Uses the stored refresh token to obtain a new session token.
    /// Should be called before token expiration to maintain session.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * No refresh token available
    /// * Network request fails
    /// * Token renewal fails
    pub async fn renew_login(&mut self) -> Result<()> {
        // `c` for cookie (headers), `p` for payload (body)
        let query = Url::parse_with_params(
            &format!("{}{}", Self::JWT_AUTH_URL, Self::JWT_ENDPOINT_RENEW),
            &[("jo", "p"), ("rto", "c"), ("i", "c")],
        )?;

        let request = self.http_client.json(query, Self::EMPTY_JSON_OBJECT);
        self.http_client.execute(request).await?;

        // When successful, the `refresh-token` cookie is set within the HTTP client's cookie store.
        Ok(())
    }

    /// Logs out and invalidates the current session.
    ///
    /// Clears:
    /// * Authentication tokens
    /// * Session cookies
    /// * User data
    ///
    /// # Errors
    ///
    /// Returns error if network request fails
    pub async fn logout(&mut self) -> Result<()> {
        let query = Url::parse(&format!(
            "{}{}",
            Self::JWT_AUTH_URL,
            Self::JWT_ENDPOINT_LOGOUT
        ))?;
        let request = self.http_client.get(query, "");
        self.http_client.execute(request).await?;
        Ok(())
    }
}
