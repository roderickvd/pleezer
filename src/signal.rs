//! System signal handling for graceful shutdown and reload.
//!
//! This module provides unified signal handling across platforms:
//! * Unix: SIGTERM, SIGHUP, and Ctrl-C (SIGINT)
//! * Windows: Ctrl-C only
//!
//! # Example
//!
//! ```no_run
//! use pleezer::signal::{Handler, ShutdownSignal};
//!
//! async fn example() {
//!     let mut signals = Handler::new().unwrap();
//!
//!     match signals.recv().await {
//!         ShutdownSignal::Interrupt | ShutdownSignal::Terminate => {
//!             println!("Shutting down...");
//!         }
//!         ShutdownSignal::Reload => {
//!             println!("Reloading configuration...");
//!         }
//!     }
//! }
//! ```

use std::fmt;

use crate::error::Result;

#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};

/// Signal that triggered a shutdown or reload.
///
/// On Unix systems, this can be:
/// * Ctrl-C (SIGINT)
/// * SIGTERM (graceful termination)
/// * SIGHUP (configuration reload)
///
/// On Windows, only Ctrl-C is supported.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[expect(clippy::module_name_repetitions)]
pub enum ShutdownSignal {
    /// Interrupt signal (Ctrl-C/SIGINT)
    Interrupt,
    /// Termination signal (SIGTERM)
    Terminate,
    /// Reload configuration signal (SIGHUP)
    Reload,
}

/// Handles system signals for graceful shutdown and reload.
///
/// Provides a unified interface for signal handling across platforms:
/// * On Unix: Handles SIGTERM, SIGHUP, and Ctrl-C
/// * On Windows: Handles only Ctrl-C
///
/// The handler is designed to be used in an async context and integrates
/// with tokio's signal handling.
pub struct Handler {
    #[cfg(unix)]
    sigterm: Signal,
    #[cfg(unix)]
    sighup: Signal,
}

impl Handler {
    /// Creates a new signal handler.
    ///
    /// # Errors
    ///
    /// Returns error if signal handlers cannot be registered.
    pub fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                sigterm: signal(SignalKind::terminate())?,
                sighup: signal(SignalKind::hangup())?,
            })
        }

        #[cfg(not(unix))]
        Ok(Self {})
    }

    /// Waits for the next signal.
    ///
    /// Returns which signal was received:
    /// * `ShutdownSignal::Interrupt` for Ctrl-C
    /// * `ShutdownSignal::Terminate` for SIGTERM (Unix only)
    /// * `ShutdownSignal::Reload` for SIGHUP (Unix only)
    ///
    /// On Windows, this only waits for Ctrl-C and always returns
    /// `ShutdownSignal::Interrupt`.
    pub async fn recv(&mut self) -> ShutdownSignal {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => ShutdownSignal::Interrupt,
                _ = self.sigterm.recv() => ShutdownSignal::Terminate,
                _ = self.sighup.recv() => ShutdownSignal::Reload,
            }
        }

        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await;
            ShutdownSignal::Interrupt
        }
    }
}

/// Formats the shutdown signal in a human-readable form.
///
/// Returns:
/// * "Ctrl+C" for [`ShutdownSignal::Interrupt`]
/// * "SIGTERM" for [`ShutdownSignal::Terminate`]
/// * "SIGHUP" for [`ShutdownSignal::Reload`]
impl fmt::Display for ShutdownSignal {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShutdownSignal::Interrupt => write!(f, "Ctrl+C"),
            ShutdownSignal::Terminate => write!(f, "SIGTERM"),
            ShutdownSignal::Reload => write!(f, "SIGHUP"),
        }
    }
}
