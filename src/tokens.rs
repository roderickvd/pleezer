use std::{
    fmt,
    num::NonZeroU64,
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserToken {
    // TODO: replace with UserId
    pub user_id: NonZeroU64,
    pub token: String,
    pub expires_at: SystemTime,
}

impl UserToken {
    #[must_use]
    pub fn time_to_live(&self) -> Duration {
        self.expires_at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        SystemTime::now() >= self.expires_at
    }
}

impl fmt::Display for UserToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.token)
    }
}
