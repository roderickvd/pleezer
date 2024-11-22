use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DeserializeFromStr, DisplayFromStr, SerializeDisplay};
use uuid::Uuid;

use super::channel::UserId;
use crate::{error::Error, track::TrackId};

/// The contents of a [`Message`] on a [`Stream`] [`Channel`] on a
/// [Deezer Connect][Connect] websocket.
///
/// [`Channel`]: ../channel/struct.Channel.html
/// [`Message`]: ../messages/enum.Message.html
/// [`Stream`]: ../channel/enum.Channel.html#variant.Stream
/// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
#[serde_as]
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contents {
    /// The [Deezer Connect][Connect] websocket [`Message`] [`Action`]
    /// that these `Contents` are for.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    /// [`Action`]: enum.Action.html
    /// [`Message`]: ../messages/enum.Message.html
    #[serde(rename = "ACTION")]
    pub action: Action,

    /// The [Deezer Connect][Connect] websocket [`Message`] [`Ident`]
    /// that these `Contents` are for.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    /// [`Action`]: enum.Ident.html
    /// [`Message`]: ../messages/enum.Message.html
    #[serde(rename = "APP")]
    pub ident: Ident,

    /// The value of these [Deezer Connect][Connect] websocket [`Message`]
    /// `Contents`.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    /// [`Message`]: ../messages/enum.Message.html
    #[serde(rename = "VALUE")]
    pub value: Value,
}

#[serde_as]
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Value {
    #[serde(rename = "USER_ID")]
    #[serde_as(as = "DisplayFromStr")]
    pub user: UserId,

    #[serde(rename = "UNIQID")]
    pub uuid: Uuid,

    #[serde(rename = "SNG_ID")]
    #[serde_as(as = "DisplayFromStr")]
    pub track_id: TrackId,
}

#[derive(Copy, Clone, Debug, SerializeDisplay, DeserializeFromStr, PartialEq, Eq, Hash)]
pub enum Action {
    Play,
}

#[derive(Copy, Clone, Debug, SerializeDisplay, DeserializeFromStr, PartialEq, Eq, Hash)]
pub enum Ident {
    Limitation,
}

impl Action {
    const PLAY: &'static str = "PLAY";
}

impl Ident {
    const LIMITATION: &'static str = "LIMITATION";
}

impl fmt::Display for Contents {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.action, self.value.track_id)
    }
}

impl fmt::Display for Action {
    /// Formats an `Action` as a wire string for use on a
    /// [Deezer Connect][Connect] websocket.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Play => write!(f, "{}", Self::PLAY),
        }
    }
}

impl FromStr for Action {
    type Err = Error;

    /// Parses a wire string `s` on a [Deezer Connect][Connect] websocket to
    /// return a variant of `Action`.
    ///
    /// The string `s` is parsed as uppercase.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let variant = match s {
            Self::PLAY => Self::Play,
            _ => return Err(Self::Err::unimplemented(format!("stream action `{s}`"))),
        };

        Ok(variant)
    }
}

impl fmt::Display for Ident {
    /// Formats an `Ident` as a wire string for use on a
    /// [Deezer Connect][Connect] websocket.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limitation => write!(f, "{}", Self::LIMITATION),
        }
    }
}

impl FromStr for Ident {
    type Err = Error;

    /// Parses a wire string `s` on a [Deezer Connect][Connect] websocket to
    /// return a variant of `Ident`.
    ///
    /// The string `s` is parsed as uppercase.
    ///
    /// [Connect]: https://en.deezercommunity.com/product-updates/try-our-remote-control-and-let-us-know-how-it-works-70079
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let variant = match s {
            Self::LIMITATION => Self::Limitation,
            _ => return Err(Self::Err::unimplemented(format!("stream action `{s}`"))),
        };

        Ok(variant)
    }
}
