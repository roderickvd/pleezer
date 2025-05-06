//! Main application entry point and runtime management.
//!
//! This module handles:
//! * Command line argument parsing
//! * Logging configuration
//! * Configuration loading
//! * Application lifecycle
//! * Connection retry logic with exponential backoff
//! * Audio quality settings:
//!   - Volume normalization
//!   - Dithering with configurable bit depth
//!   - Psychoacoustic noise shaping
//!
//! * Audio content:
//!   - Songs
//!   - Podcast episodes
//!   - Live radio
//!
//! # Runtime Behavior
//!
//! The application:
//! 1. Loads and validates configuration
//! 2. Establishes Deezer connection
//! 3. Maintains connection with automatic retry on failures:
//!    * Uses exponential backoff with jitter
//!    * Makes up to 10 retry attempts
//!    * Backs off between 100ms and 10s
//! 4. Handles graceful shutdown
//!
//! # Error Handling
//!
//! Errors are handled at different levels:
//! * Configuration errors terminate immediately
//! * Authentication errors terminate immediately
//! * Network errors trigger automatic retry with backoff
//! * ARL expiration triggers immediate retry
//! * Other errors are logged and may trigger retry
//!
//! # Retry Behavior
//!
//! The retry logic uses exponential backoff with the following parameters:
//! * Maximum 5 retry attempts
//! * Initial backoff of 100ms
//! * Maximum backoff of 10 seconds
//! * Random jitter between attempts

use std::{env, fs, path::Path, process, time::Duration};

use clap::{Parser, ValueHint, command};
use exponential_backoff::Backoff;
use log::{LevelFilter, debug, error, info, trace, warn};

use pleezer::{
    arl::Arl,
    config::{Config, Credentials},
    decrypt,
    error::{Error, ErrorKind, Result},
    player::Player,
    protocol::connect::{DeviceType, Percentage},
    remote,
    signal::{self, ShutdownSignal},
    uuid::Uuid,
};

/// Build profile indicator for logging.
///
/// Shows "debug" when built without optimizations.
#[cfg(debug_assertions)]
const BUILD_PROFILE: &str = "debug";

/// Build profile indicator for logging.
///
/// Shows "release" when built with optimizations.
#[cfg(not(debug_assertions))]
const BUILD_PROFILE: &str = "release";

/// Group name for mutually exclusive logging options.
///
/// Used by clap to ensure -q (quiet) and -v (verbose) flags
/// cannot be used together.
const ARGS_GROUP_LOGGING: &str = "logging";

/// Number of retry attempts before giving up.
///
/// After this many failed connection attempts, the application will terminate
/// with an error instead of continuing to retry.
const BACKOFF_ATTEMPTS: u32 = 10;

/// Minimum duration to wait between retry attempts.
///
/// The first retry will wait at least this long, with subsequent retries
/// increasing exponentially up to MAX_BACKOFF.
const MIN_BACKOFF: Duration = Duration::from_millis(100);

/// Maximum duration to wait between retry attempts.
///
/// Backoff periods will not exceed this duration, even with
/// exponential increases.
const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// Command line arguments as parsed by `clap`.
///
/// Provides configuration options for:
/// * Authentication (secrets file)
/// * Device identification (name, type)
/// * Audio settings:
///   - Output device selection
///   - Volume normalization
///   - Dithering configuration
///   - Noise shaping profiles
/// * Connection behavior (interruptions, binding)
/// * Debug features (logging, eavesdropping)
///
/// All options can be set via environment variables with
/// the `PLEEZER_` prefix.
#[derive(Clone, Debug, Default, PartialEq, PartialOrd, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the secrets file
    ///
    /// Keep this file secure and private, as it contains sensitive information
    /// that can grant access to your Deezer account.
    #[arg(short, long, value_name = "FILE", value_hint = ValueHint::FilePath, default_value_t = String::from("secrets.toml"), env = "PLEEZER_SECRETS")]
    secrets: String,

    /// Set the player's name as shown to Deezer clients
    ///
    /// If not specified, uses the system hostname.
    #[arg(short, long, value_hint = ValueHint::Hostname, env = "PLEEZER_NAME")]
    name: Option<String>,

    /// Set the device type to identify as to Deezer
    ///
    /// This affects how the device appears in Deezer apps.
    /// Values: web, mobile, tablet, desktop
    #[arg(long, default_value_t = DeviceType::Web, env = "PLEEZER_DEVICE_TYPE")]
    device_type: DeviceType,

    /// Select the audio output device
    ///
    /// Format: [<host>][|<device>][|<sample rate>][|<sample format>]
    /// Use "?" to list available stereo 44.1/48 kHz output devices.
    /// If omitted, uses the system default output device.
    #[arg(short, long, default_value = None, env = "PLEEZER_DEVICE")]
    device: Option<String>,

    /// Enable volume normalization
    ///
    /// Normalizes volume across tracks to provide consistent listening levels.
    #[arg(long, default_value_t = false, env = "PLEEZER_NORMALIZE_VOLUME")]
    normalize_volume: bool,

    /// Enable loudness compensation (ISO 226:2013)
    ///
    /// Applies frequency-dependent gain to match human hearing sensitivity.
    /// Particularly useful for low-volume listening.
    #[arg(long, default_value_t = false, env = "PLEEZER_LOUDNESS")]
    loudness: bool,

    /// Set initial volume level (0-100)
    ///
    /// Applied when no volume is reported by Deezer client or when reported as maximum.
    /// Useful for clients that don't correctly set volume levels.
    #[arg(
        long,
        value_name = "PERCENTAGE",
        value_parser = clap::value_parser!(u8).range(0..=100),
        env = "PLEEZER_INITIAL_VOLUME"
    )]
    initial_volume: Option<u8>,

    /// Set dither bit depth based on DAC linearity (ENOB)
    ///
    /// Set to effective number of bits from DAC measurements, or 0 to disable dithering.
    /// Default: 15.5 bits for 16-bit DAC, 19.5 bits for 32-bit DAC.
    #[arg(
        long,
        value_name = "BITS",
        value_parser = clap::value_parser!(f32),
        env = "PLEEZER_DITHER_BITS"
    )]
    dither_bits: Option<f32>,

    /// Set noise shaping level
    ///
    /// Level 2 (default) offers mild noise shaping (~8 dB ultrasonic rise) suitable for most music.
    ///
    /// * 0: No shaping (plain TPDF dither) - safest, recommended for podcasts
    /// * 1: Very mild shaping (~5 dB ultrasonic rise)
    /// * 2: Mild shaping (~8 dB rise) - recommended default for most music
    /// * 3: Moderate shaping (~12 dB rise) - can benefit classical/jazz/ambient
    /// * 4-7: Aggressive shaping - not recommended due to high ultrasonic energy that may:
    ///   - Stress tweeters and headphone drivers
    ///   - Cause intermodulation distortion
    ///   - Create fatiguing sound
    #[arg(
        long,
        value_parser = clap::value_parser!(u8).range(0..=7),
        default_value_t = 2,
        env = "PLEEZER_NOISE_SHAPING",
        verbatim_doc_comment
    )]
    noise_shaping: u8,

    /// Maximum RAM (in MB) to use for storing audio files in memory
    ///
    /// If not specified or if a track exceeds this limit, temporary files will be used.
    #[arg(
        long,
        value_name = "MEGABYTES",
        value_parser = clap::value_parser!(u64).range(1..=1024*1024), // Allow 1MB to 1TB
        env = "PLEEZER_MAX_RAM"
    )]
    max_ram: Option<u64>,

    /// Prevent other clients from taking over the connection
    ///
    /// By default, other clients can interrupt and take control of playback.
    #[arg(long, default_value_t = false, env = "PLEEZER_NO_INTERRUPTIONS")]
    no_interruptions: bool,

    /// Address to bind outgoing connections to
    ///
    /// Defaults to "0.0.0.0" (IPv4 any address) since Deezer services are IPv4-only
    /// Can be set to a specific IPv4 or IPv6 address to control which network interface
    /// is used for outgoing connections, for example when using tunneling or specific
    /// routing requirements.
    #[arg(long, default_value = "0.0.0.0", env = "PLEEZER_BIND")]
    bind: String,

    /// Script to execute when events occur
    #[arg(long, value_hint = ValueHint::ExecutablePath, env = "PLEEZER_HOOK")]
    hook: Option<String>,

    /// Suppress all output except warnings and errors
    #[arg(short, long, default_value_t = false, group = ARGS_GROUP_LOGGING, env = "PLEEZER_QUIET")]
    quiet: bool,

    /// Enable verbose logging
    ///
    /// Use -v for debug logging
    /// Use -vv for trace logging
    #[arg(short, long, action = clap::ArgAction::Count, group = ARGS_GROUP_LOGGING, env = "PLEEZER_VERBOSE")]
    verbose: u8,

    /// Monitor the Deezer Connect websocket without participating
    ///
    /// A development tool that observes websocket traffic. Requires verbose
    /// logging (-v or -vv). For best results, use trace logging (-vv).
    #[arg(
        long,
        default_value_t = false,
        requires = "verbose",
        env = "PLEEZER_EAVESDROP"
    )]
    eavesdrop: bool,
}

/// Initialize logging system.
///
/// Configures logging based on command line arguments and environment:
/// * `-q` sets Warning level
/// * `-v` sets Debug level
/// * `-vv` sets Trace level
/// * `RUST_LOG` environment variable provides defaults
/// * External crates are limited to Warning level
///
/// # Arguments
///
/// * `config` - Command line arguments containing logging options
///
/// # Panics
///
/// Panics if logger is already initialized.
fn init_logger(config: &Args) {
    let mut logger = env_logger::Builder::from_env(
        // Note: if you change the default logging level here, then you should
        // probably also change the verbosity levels below.
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let mut external_level = LevelFilter::Error;
    if config.quiet || config.verbose > 0 {
        let level = match config.verbose {
            0 => {
                // Quiet and verbose are mutually exclusive, and `verbose` is 0
                // by default. So this arm means: quiet mode.
                LevelFilter::Warn
            }
            1 => LevelFilter::Debug,
            _ => LevelFilter::max(),
        };

        // Filter log messages of pleezer.
        logger.filter_module(module_path!(), level);

        if level == LevelFilter::Trace {
            // Filter log messages of external crates.
            external_level = LevelFilter::max();
        }
    };

    // Filter log messages of external crates.
    for external_module in [
        "symphonia",
        "symphonia_bundle_flac",
        "symphonia_bundle_mp3",
        "symphonia_codec_aac",
        "symphonia_codec_pcm",
        "symphonia_core",
        "symphonia_format_isomp4",
        "symphonia_format_riff",
        "symphonia_metadata",
        "symphonia_utils_xiph",
    ] {
        logger.filter_module(external_module, external_level);
    }

    logger.init();
}

/// Parse the secrets file into a configuration value.
///
/// # Security
///
/// To prevent resource exhaustion attacks:
/// * File size is limited to 1024 bytes
/// * Contents must be valid UTF-8
/// * Must be valid TOML format
///
/// # Arguments
///
/// * `secrets` - Path to the secrets file
///
/// # Errors
///
/// Returns error if:
/// * File cannot be read
/// * File exceeds size limit
/// * Content isn't valid UTF-8
/// * Content isn't valid TOML
fn parse_secrets(secrets: impl AsRef<Path>) -> Result<toml::Value> {
    // Prevent out-of-memory condition: secrets file should be small.
    let attributes = fs::metadata(&secrets)?;
    let file_size = attributes.len();
    if file_size > 4096 {
        return Err(Error::out_of_range(format!(
            "{} too large: {file_size} bytes",
            secrets.as_ref().to_string_lossy()
        )));
    }

    let contents = fs::read_to_string(&secrets)?;
    contents.parse::<toml::Value>().map_err(|e| {
        Error::invalid_argument(format!(
            "{} format invalid: {e}",
            secrets.as_ref().to_string_lossy()
        ))
    })
}

/// Main application loop.
///
/// Handles the core application lifecycle:
/// 1. Loads configuration
/// 2. Sets up player and client
/// 3. Manages connection lifecycle
/// 4. Implements retry with jitter
/// 5. Handles system signals (Ctrl-C, SIGTERM, SIGHUP)
///
/// # Arguments
///
/// * `args` - Parsed command line arguments
///
/// # Returns
///
/// Returns the signal that triggered the shutdown, or an error if one occurred.
/// SIGHUP triggers a configuration reload and restart.
///
/// # Errors
///
/// Returns error if:
/// * Configuration is invalid
/// * Authentication fails
/// * Device initialization fails
/// * Too many devices are registered
/// * Resource limits are exceeded
/// * Unrecoverable network error occurs
///
/// Network errors that might be temporary will trigger retry instead.
async fn run(args: Args) -> Result<ShutdownSignal> {
    if args
        .dither_bits
        .is_some_and(|bits| !(0.0..=24.0).contains(&bits))
    {
        return Err(Error::invalid_argument(
            "dither bits must be between 0 and 24",
        ));
    }

    if args.device.as_ref().is_some_and(|device| device == "?") {
        // List available devices and exit.
        let devices = Player::enumerate_devices();
        if devices.is_empty() {
            return Err(Error::not_found(
                "no stereo 44.1/48 kHz output devices found",
            ));
        }

        info!("available stereo 44.1/48 kHz output devices:");
        for device in devices {
            info!("- {device}");
        }
        return Ok(ShutdownSignal::Interrupt);
    }

    if let Ok(proxy) = env::var("HTTPS_PROXY") {
        info!("using proxy: {proxy}");
    }

    let config = {
        // Get the credentials from the secrets file.
        info!("parsing secrets from {}", args.secrets);
        let secrets = parse_secrets(args.secrets)?;

        let credentials = match secrets.get("arl").and_then(|value| value.as_str()) {
            Some(arl) => {
                let result = arl.parse::<Arl>()?;
                info!("using arl from secrets file");
                Credentials::Arl(result)
            }
            None => {
                let email = secrets
                    .get("email")
                    .and_then(|email| email.as_str())
                    .ok_or_else(|| Error::unauthenticated("email not found"))?;
                let password = secrets
                    .get("password")
                    .and_then(|password| password.as_str())
                    .ok_or_else(|| Error::unauthenticated("password not found"))?;

                Credentials::Login {
                    email: email.to_string(),
                    password: password.to_string(),
                }
            }
        };

        let bf_secret = match secrets.get("bf_secret").and_then(|value| value.as_str()) {
            Some(value) => {
                let key = value.parse::<decrypt::Key>()?;
                Some(key)
            }
            None => None,
        };

        let app_name = env!("CARGO_PKG_NAME").to_owned();
        let app_version = env!("CARGO_PKG_VERSION").to_owned();
        let app_lang = "en".to_owned();

        let device_id = *machine_uid::get()
            .and_then(|uid| uid.parse().map_err(Into::into))
            .unwrap_or_else(|_| {
                warn!("could not get machine uuid, using random device id");
                Uuid::fast_v4()
            });
        trace!("device uuid: {device_id}");

        // Additional `User-Agent` string checks on top of what
        // `reqwest::HeaderValue` already checks.
        let illegal_chars = |chr| chr == '/' || chr == ';';
        if app_name.is_empty()
            || app_name.contains(illegal_chars)
            || app_version.is_empty()
            || app_version.contains(illegal_chars)
            || app_lang.chars().count() != 2
            || app_lang.contains(illegal_chars)
        {
            return Err(Error::invalid_argument(format!(
                "application name, version and/or language invalid (\"{app_name}\"; \"{app_version}\"; \"{app_lang}\")"
            )));
        }

        let os_name = match std::env::consts::OS {
            "macos" => "osx",
            other => other,
        };

        let os_version = match std::env::consts::OS {
            "linux" => sysinfo::System::kernel_version(),
            _ => sysinfo::System::os_version(),
        }
        .unwrap_or("0".to_string());
        if os_name.is_empty()
            || os_name.contains(illegal_chars)
            || os_version.is_empty()
            || os_version.contains(illegal_chars)
        {
            return Err(Error::invalid_argument(format!(
                "os name and/or version invalid (\"{os_name}\"; \"{os_version}\")"
            )));
        }

        // Set `User-Agent` to be served like Deezer on desktop.
        let user_agent = format!(
            "{app_name}/{app_version} (Rust; {os_name}/{os_version}; like Desktop; {app_lang})"
        );
        trace!("user agent: {user_agent}");

        // Deezer on desktop uses a new `cid` on every start.
        let client_id = fastrand::usize(100_000_000..=999_999_999);
        trace!("client id: {client_id}");

        Config {
            app_name: app_name.clone(),
            app_version,
            app_lang,

            device_id,
            device_type: args.device_type,
            device_name: args
                .name
                .or_else(|| sysinfo::System::host_name().clone())
                .unwrap_or_else(|| app_name.clone()),

            interruptions: !args.no_interruptions,

            normalization: args.normalize_volume,
            loudness: args.loudness,
            initial_volume: args
                .initial_volume
                .map(|volume| Percentage::from_percent(volume as f32)),

            dither_bits: args.dither_bits,
            noise_shaping: args.noise_shaping,

            // Convert MB to bytes
            max_ram: args.max_ram.map(|mb| mb * 1024 * 1024),
            hook: args.hook,

            client_id,
            user_agent,

            credentials,
            bf_secret,

            eavesdrop: args.eavesdrop,
            bind_address: args.bind.parse()?,
        }
    };

    let player = Player::new(&config, args.device.as_deref().unwrap_or_default()).await?;
    let mut client = remote::Client::new(&config, player)?;
    let mut signals = signal::Handler::new()?;

    // Main application loop. This restarts the new remote client when it gets disconnected for
    // whatever reason. This could be from a network failure or an arl that expired. In this case,
    // we try to recover from the error by restarting the client. If the error is a permission
    // we bail out, because the user is not be able to login.
    loop {
        tokio::select! {
            // Prioritize shutdown signals.
            biased;

            signal = signals.recv() => {
                match signal {
                    ShutdownSignal::Interrupt | ShutdownSignal::Terminate => {
                        info!("received {signal}, shutting down");
                    }
                    ShutdownSignal::Reload => {
                        info!("received {signal}, restarting client");
                    }
                }
                client.stop().await;
                break Ok(signal);
            }

            result = async {
                for (i, backoff) in Backoff::new(BACKOFF_ATTEMPTS, MIN_BACKOFF, MAX_BACKOFF).into_iter().enumerate() {
                    match client.start().await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            match e.kind {
                                // Bail out if the user is:
                                // - not able to login
                                // - not allowed to use remote control
                                ErrorKind::PermissionDenied |
                                // - using too many devices
                                ErrorKind::ResourceExhausted |
                                // - on a free-tier account
                                ErrorKind::Unimplemented => {
                                    return Err(e);
                                },
                                ErrorKind::DeadlineExceeded => {
                                    // Retry when the arl is expired.
                                    warn!("{e}");
                                    return Ok(());
                                }
                                _ => match backoff {
                                    // Retry `BACKOFF_ATTEMPTS` times with exponential backoff
                                    // on network errors.
                                    Some(duration) => {
                                        error!("{e}; retrying in {duration:?} ({}/{BACKOFF_ATTEMPTS})", i+1);
                                        tokio::time::sleep(duration).await;
                                    }
                                    // Bail out if we have exhausted all retries.
                                    None => return Err(e),
                                }
                            }
                        },
                    }
                }

                Ok(())
            } => {
                match result {
                    Ok(()) => { info!("restarting client"); }
                    Err(e) => break Err(e),
                }
            }
        }
    }
}

/// Application entry point.
///
/// Sets up the environment and manages the application lifecycle:
/// 1. Parses command line arguments
/// 2. Initializes logging
/// 3. Runs main loop with restart support
/// 4. Handles shutdown conditions:
///    - Clean exit on SIGTERM/Ctrl-C
///    - Restart on SIGHUP
///    - Error exit on failures
///
/// Exits with status code:
/// - 0 for clean shutdown
/// - 1 if an error occurs
#[tokio::main]
async fn main() {
    // `clap` handles our command line arguments and help text.
    let args = Args::parse();
    init_logger(&args);

    // Dump command line arguments before we do anything more.
    // This aids in debugging of whatever comes next.
    debug!("Command {:#?}", args);

    let cmd = command!();
    let name = cmd.get_name().to_string();

    let mut version = cmd.get_version().unwrap_or("UNKNOWN").to_string();
    if let Some(hash) = option_env!("PLEEZER_COMMIT_HASH") {
        version.push_str(&format!(".{hash}"));
    }
    if let Some(date) = option_env!("PLEEZER_COMMIT_DATE") {
        version.push_str(&format!(" ({date})"));
    }

    info!("starting {name}/{version}; {BUILD_PROFILE}");

    loop {
        match run(args.clone()).await {
            Ok(signal) => {
                if signal == ShutdownSignal::Reload {
                    continue;
                }
                info!("shut down gracefully");
                process::exit(0);
            }
            Err(e) => {
                error!("{e}");
                process::exit(1);
            }
        }
    }
}
