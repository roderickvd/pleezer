//! Numbers are parsed and stored in 64-bit format, because [JSON] does not
//! distinguish between different sizes of numbers.

pub mod arl;
pub mod list_data;
pub mod user_data;
pub mod user_radio;

pub use arl::Arl;
pub use list_data::{ListData, Queue};
pub use user_data::{MediaUrl, UserData};
pub use user_radio::UserRadio;

use std::{collections::HashMap, ops::Deref, str::FromStr};

use serde::Deserialize;
use serde_with::serde_as;

pub trait Method {
    const METHOD: &'static str;
}

#[serde_as]
#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(untagged)]
pub enum Response<T> {
    Paginated {
        #[serde_as(as = "serde_with::Seq<(_, _)>")]
        error: HashMap<String, serde_json::Value>,
        results: Paginated<T>,
    },

    Unpaginated {
        #[serde_as(as = "serde_with::Seq<(_, _)>")]
        error: HashMap<String, serde_json::Value>,
        #[serde_as(as = "serde_with::OneOrMany<_>")]
        results: Vec<T>,
    },
}

impl<T> Response<T> {
    #[must_use]
    pub fn first(&self) -> Option<&T> {
        self.all().first()
    }

    #[must_use]
    pub fn all(&self) -> &Vec<T> {
        match self {
            Self::Paginated { results, .. } => &results.data,
            Self::Unpaginated { results, .. } => results,
        }
    }
}

#[serde_as]
#[derive(Clone, PartialEq, Deserialize, Debug)]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub count: u64,
    pub total: u64,
    pub filtered_count: u64,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Debug, Hash)]
pub struct StringOrUnknown(pub String);

impl Deref for StringOrUnknown {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for StringOrUnknown {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl Default for StringOrUnknown {
    fn default() -> Self {
        Self(String::from("UNKNOWN"))
    }
}
