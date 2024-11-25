use std::{str::FromStr, time::SystemTime};

use md5::{Digest, Md5};
use reqwest::{
    self,
    header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    Url,
};
use serde::Deserialize;

use crate::{
    arl::Arl,
    config::{Config, Credentials},
    error::{Error, ErrorKind, Result},
    http::Client as HttpClient,
    protocol::{
        connect::{
            queue::{self, TrackType},
            AudioQuality, UserId,
        },
        gateway::{self, Queue, UserData},
    },
    tokens::UserToken,
};

pub struct Gateway {
    http_client: HttpClient,
    // TODO : we probably don't need to retain all user data, all the time
    //       keep what we need here in the gateway, and send the rest off into
    //       a token object
    user_data: Option<UserData>,
    client_id: usize,
}

impl Gateway {
    /// The URL of the Deezer cookie origin.
    ///
    /// This URL is not entirely correct, as the cookies could come from
    /// `connect.deezer.com` or `www.deezer.com` as well. What
    /// matters is that the domain matches with `deezer.com`.
    const COOKIE_ORIGIN: &'static str = "https://www.deezer.com";

    /// The URL of the Deezer gateway.
    const GATEWAY_URL: &'static str = "https://www.deezer.com/ajax/gw-light.php";

    /// The Deezer gateway version.
    const GATEWAY_VERSION: &'static str = "1.0";

    /// The Deezer gateway input type.
    const GATEWAY_INPUT: usize = 3;

    /// The Deezer API client ID for authenticating.
    const OAUTH_CLIENT_ID: usize = 447_462;

    /// The Deezer API salt for the password.
    const OAUTH_SALT: &'static str = "a83bf7f38ad2f137e444727cfc3775cf";

    /// The Deezer API URL that will be used to get the session ID.
    const OAUTH_SID_URL: &'static str = "https://connect.deezer.com/oauth/auth.php";

    /// The Deezer API authentication URL.
    const OAUTH_LOGIN_URL: &'static str = "https://connect.deezer.com/oauth/user_auth.php";

    /// The `Content-Type` header value for the Deezer gateway requests.
    ///
    /// Although the bodies of all gateway requests are JSON, the
    /// `Content-Type` is not.
    const PLAIN_TEXT_CONTENT: HeaderValue = HeaderValue::from_static("text/plain;charset=UTF-8");

    /// An empty JSON object that is used as the default body for the Deezer
    /// API gateway requests.
    const EMPTY_JSON_OBJECT: &'static str = "{}";

    /// The cookie origin for the Deezer API as a `reqwest::Url`.
    ///
    /// # Panics
    ///
    /// Will panic if the URL is invalid.
    fn cookie_origin() -> reqwest::Url {
        reqwest::Url::parse(Self::COOKIE_ORIGIN).expect("invalid cookie origin")
    }

    /// Creates a new `reqwest::cookie::Jar` containing the necessary cookies
    /// for the Deezer API.
    fn cookie_jar(config: &Config) -> reqwest::cookie::Jar {
        let cookie_jar = reqwest::cookie::Jar::default();
        let cookie_origin = Self::cookie_origin();

        let lang_cookie = format!(
            "dz_lang={}; Domain=deezer.com; Path=/; Secure; HttpOnly",
            &config.app_lang
        );
        cookie_jar.add_cookie_str(&lang_cookie, &cookie_origin);

        if let Credentials::Arl(ref arl) = config.credentials {
            let arl_cookie = format!("arl={arl}; Domain=deezer.com; Path=/; Secure; HttpOnly");
            cookie_jar.add_cookie_str(&arl_cookie, &cookie_origin);
        }

        cookie_jar
    }

    /// TODO
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - no valid `User-Agent` can be created out of the `config` fields
    /// - no valid OS name and/or version can be detected
    /// - no valid cookies can be created out of the `arl` and/or `config` fields
    pub fn new(config: &Config) -> Result<Self> {
        // Create a new cookie jar and put the cookies in.
        let cookie_jar = Self::cookie_jar(config);
        let http_client = HttpClient::with_cookies(config, cookie_jar)?;

        Ok(Self {
            client_id: config.client_id,
            http_client,
            user_data: None,
        })
    }

    pub fn cookies(&self) -> Option<HeaderValue> {
        self.http_client
            .cookie_jar
            .as_ref()
            .and_then(|jar| jar.cookies(&Self::cookie_origin()))
    }

    /// TODO
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the `arl` is invalid or expired
    /// - the HTTP request failed
    pub async fn refresh(&mut self) -> Result<()> {
        // Send an empty JSON map
        match self
            .request::<gateway::UserData>(Self::EMPTY_JSON_OBJECT, None)
            .await
        {
            Ok(response) => {
                if let Some(data) = response.first() {
                    if !data.gatekeeps.remote_control {
                        return Err(Error::permission_denied(
                            "remote control is disabled for this account".to_string(),
                        ));
                    }
                    if data.user.options.too_many_devices {
                        return Err(Error::permission_denied(
                            "too many devices; remove one or more in your account settings"
                                .to_string(),
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

    /// Performs a request to the Deezer API gateway.
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - no valid [`Url`] can be created out of the session data
    /// - the HTTP request fails
    /// - the HTTP response cannot be parsed as [JSON]
    pub async fn request<T>(
        &mut self,
        body: impl Into<reqwest::Body>,
        headers: Option<HeaderMap>,
    ) -> Result<gateway::Response<T>>
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
        let mut request = self.http_client.post(url, body);

        let request_headers = request.headers_mut();
        request_headers.try_insert(CONTENT_TYPE, Self::PLAIN_TEXT_CONTENT)?;

        // Add any headers that were passed in.
        if let Some(headers) = headers {
            request_headers.extend(headers);
        }

        let response = self.http_client.execute(request).await?;
        let body = response.text().await?;

        let result: gateway::Response<T> = match serde_json::from_str(&body) {
            Ok(result) => {
                trace!("{}: {result:#?}", T::METHOD);
                result
            }
            Err(e) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    trace!("{}: {json:#?}", T::METHOD);
                } else {
                    error!("{}: failed parsing response ({e:?})", T::METHOD);
                    trace!("{body}");
                }
                return Err(e.into());
            }
        };

        Ok(result)
    }

    #[must_use]
    pub fn license_token(&self) -> Option<&str> {
        self.user_data
            .as_ref()
            .map(|data| data.user.options.license_token.as_str())
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        if let Some(data) = &self.user_data {
            return data.user.options.expiration_timestamp >= data.user.options.timestamp;
        }

        true
    }

    #[must_use]
    pub fn expires_at(&self) -> SystemTime {
        if let Some(data) = &self.user_data {
            return data.user.options.expiration_timestamp;
        }

        SystemTime::now()
    }

    pub fn set_user_data(&mut self, data: UserData) {
        self.user_data = Some(data);
    }

    #[must_use]
    pub fn user_data(&self) -> Option<&gateway::UserData> {
        self.user_data.as_ref()
    }

    /// The [`AudioQuality`] that the user has set for casting.
    pub fn audio_quality(&self) -> Option<AudioQuality> {
        self.user_data.as_ref().and_then(|data| {
            AudioQuality::from_str(&data.user.audio_settings.connected_device_streaming_preset).ok()
        })
    }

    /// Whether the user has enabled normalization.
    pub fn normalization(&self) -> Option<bool> {
        self.user_data
            .as_ref()
            .map(|data| data.user.settings.site.player_normalize)
    }

    /// The reference level for normalization.
    ///
    /// This function truncates the value to an `i8` because the API could return
    /// a value that is out of bounds.
    #[expect(clippy::cast_possible_truncation)]
    pub fn target_gain(&self) -> Option<i8> {
        self.user_data.as_ref().map(|data| {
            data.gain
                .target
                .clamp(i64::from(i8::MIN), i64::from(i8::MAX)) as i8
        })
    }

    /// The user's account name.
    pub fn user_name(&self) -> Option<&str> {
        self.user_data.as_ref().map(|data| data.user.name.as_str())
    }

    // The URL to use for media requests.
    pub fn media_url(&self) -> Option<&str> {
        self.user_data.as_ref().map(|data| data.media_url.as_str())
    }

    /// Converts a list of tracks from the Deezer API to a [`Queue`].
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the list contains an invalid track ID
    /// - the HTTP request fails
    /// - the HTTP response cannot be parsed as [JSON]
    pub async fn list_to_queue(&mut self, list: &queue::List) -> Result<Queue> {
        let track_list = gateway::list_data::Request {
            track_ids: list
                .tracks
                .iter()
                .map(|track| {
                    let track_type = track.typ.enum_value_or_default();
                    if track_type == TrackType::TRACK_TYPE_SONG {
                        track.id.parse().map_err(Into::into)
                    } else {
                        Err(Error::unimplemented(format!(
                            "{track_type:?} not yet implemented"
                        )))
                    }
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
        };

        let body = serde_json::to_string(&track_list)?;
        match self.request::<gateway::ListData>(body, None).await {
            Ok(response) => Ok(response.all().clone()),
            Err(e) => Err(e),
        }
    }

    /// Gets a Flow playlist for the user.
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the HTTP request fails
    /// - the HTTP response cannot be parsed as [JSON]
    pub async fn user_radio(&mut self, user_id: UserId) -> Result<Queue> {
        let request = gateway::user_radio::Request { user_id };
        let body = serde_json::to_string(&request)?;
        match self.request::<gateway::UserRadio>(body, None).await {
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

    /// Get the ARL (Authentication Request Link) from the Deezer API.
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the HTTP request fails
    /// - the HTTP response cannot be parsed as [JSON]
    /// - the ARL cannot be parsed
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

    /// Get the user token that is used for remote control.
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the user data is not available
    /// - the user data does not allow remote control
    /// - the user data has too many devices
    pub async fn user_token(&mut self) -> Result<UserToken> {
        if self.is_expired() {
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

    pub fn flush_user_token(&mut self) {
        // Force refreshing user data, but do not set `user_data` to `None` so
        // so we can continue using the `api_token` it contains.
        if let Some(ref mut data) = self.user_data {
            data.user.options.expiration_timestamp = SystemTime::now();
        }
    }

    /// Log in to the Deezer API to get an ARL (Authentication Request Link).
    ///
    /// # Errors
    ///
    /// Will return `Err` if:
    /// - the email or password is incorrect
    /// - the HTTP request fails
    /// - the HTTP response cannot be parsed as [JSON]
    /// - the ARL cannot be parsed
    pub async fn login(&mut self, email: &str, password: &str) -> Result<Arl> {
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
        let _ = self.http_client.execute(request).await?;

        // Then login and get an access token.
        let query = Url::parse(&format!(
            "{}?app_id={}&login={email}&password={password:x}&hash={hash:x}",
            Self::OAUTH_LOGIN_URL,
            Self::OAUTH_CLIENT_ID,
        ))?;

        let request = self.http_client.get(query, "");
        let response = self.http_client.execute(request).await?;

        let json = response.json::<serde_json::Value>().await?;
        let access_token = json
            .get("access_token")
            .and_then(|token| token.as_str())
            .ok_or_else(|| Error::permission_denied("email or password incorrect".to_string()))?;

        // Finally use the access token to get an ARL.
        self.get_arl(access_token).await
    }
}
