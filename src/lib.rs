//! Headless streaming player for the Deezer Connect protocol.
//!
//! **pleezer** is a library and application that implements the Deezer Connect protocol,
//! enabling remote-controlled audio playback of Deezer content. It provides:
//!
//! # Core Features
//!
//! * **Remote Control**: Acts as a receiver for Deezer Connect, allowing control from
//!   official Deezer apps
//! * **Audio Playback**: High-quality audio streaming with gapless playback support
//! * **Format Support**: Handles MP3 and FLAC formats based on subscription level
//! * **Audio Processing**:
//!   - Volume normalization with configurable target gain
//!   - High-quality dithering with psychoacoustic noise shaping
//!   - Configurable for different DAC capabilities
//!
//! # Architecture
//!
//! The library is organized into several key modules:
//!
//! * **Connection Management**
//!   - [`http`]: Manages HTTP connections and cookies
//!   - [`gateway`]: Handles API authentication and requests
//!   - [`remote`]: Implements Deezer Connect protocol
//!
//! * **Audio Processing**
//!   - [`audio_file`]: Unified interface for audio stream handling
//!   - [`decrypt`]: Handles encrypted content
//!   - [`decoder`]: Audio format decoding
//!   - [`normalize`]: Audio leveling and dynamic range control
//!   - [`dither`]: High-quality dithering and noise shaping
//!   - [`volume`]: Volume control with dithering integration
//!   - [`player`]: Controls audio playback and queues
//!   - [`ringbuf`]: Ring buffer for audio processing
//!   - [`track`]: Manages track metadata and downloads
//!
//! * **Authentication**
//!   - [`arl`]: ARL token management
//!   - [`tokens`]: Session token handling
//!
//! * **Configuration**
//!   - [`config`]: Application settings
//!   - [`proxy`]: Network proxy support
//!
//! * **Protocol**
//!   - [`events`]: Event system for state changes
//!   - [`protocol`]: Deezer Connect message types
//!
//! * **System Integration**
//!   - [`signal`]: Signal handling (SIGTERM, SIGHUP)
//!   - [`mod@error`]: Error types and handling
//!   - [`util`]: General helper functions
//!   - [`uuid`]: UUID generation
//!
//! # Example
//!
//! ```rust,no_run
//! use pleezer::{config::Config, player::Player, remote::Client};
//!
//! async fn example() -> pleezer::error::Result<()> {
//!     // Create player with configuration
//!     let config = Config::new()?;
//!     let player = Player::new(&config, "").await?;
//!
//!     // Create and start client
//!     let mut client = Client::new(&config, player)?;
//!     client.start().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Protocol Documentation
//!
//! For details on the Deezer Connect protocol implementation, see the
//! [`protocol`] and [`remote`] modules.
//!
//! # Error Handling
//!
//! Errors are handled through the types in the [`mod@error`] module, with
//! most functions returning [`Result`](error::Result).
//!
//! # Signal Handling
//!
//! The application responds to system signals:
//! * SIGTERM/Ctrl-C: Graceful shutdown
//! * SIGHUP: Configuration reload
//!
//! See the [`signal`] module for details.
//!
//! # Concurrency
//!
//! The library uses async/await for concurrency and is designed to work with
//! the Tokio async runtime. Most operations are asynchronous and can run
//! concurrently.

#![deny(clippy::all)]
#![doc(test(attr(ignore)))]
#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![warn(rust_2018_idioms)]
#![warn(future_incompatible)]

#[macro_use]
extern crate log;

pub mod arl;
pub mod audio_file;
pub mod config;
pub mod decoder;
pub mod decrypt;
pub mod dither;
pub mod error;
pub mod events;
pub mod gateway;
pub mod http;
pub mod normalize;
pub mod player;
pub mod protocol;
pub mod proxy;
pub mod remote;
pub mod ringbuf;
pub mod signal;
pub mod tokens;
pub mod track;
pub mod util;
pub mod uuid;
pub mod volume;

pub use uuid::Uuid;
