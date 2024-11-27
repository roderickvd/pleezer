use serde::Deserialize;
use veil::Redact;

use super::Method;

impl Method for Arl {
    const METHOD: &'static str = "user.getArl";
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Redact, Hash)]
#[redact(all)]
pub struct Arl(pub String);
