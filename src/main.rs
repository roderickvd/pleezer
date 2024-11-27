use std::{env, fs, path::Path, process, time::Duration};

use clap::{command, Parser, ValueHint};
use log::{debug, error, info, trace, warn, LevelFilter};
use rand::Rng;
use uuid::Uuid;

use pleezer::{
    arl::Arl,
    config::{Config, Credentials},
    decrypt,
    error::{Error, ErrorKind, Result},
    player::Player,
    protocol::connect::DeviceType,
    rand::with_rng,
    remote,
};

/// Profile to display when not built in release mode.
#[cfg(debug_assertions)]
const BUILD_PROFILE: &str = "debug";
/// Profile to display when not built release mode.
#[cfg(not(debug_assertions))]
const BUILD_PROFILE: &str = "release";

/// Group name for mutually exclusive logging options.
const ARGS_GROUP_LOGGING: &str = "logging";

/// Command line arguments as parsed by `clap`.
#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the secrets file
    ///
    /// Keep this file secure and private, as it contains sensitive information
    /// that can grant access to your Deezer account.
    #[arg(short, long, value_name = "FILE", value_hint = ValueHint::FilePath, default_value_t = String::from("secrets.toml"), env = "PLEEZER_SECRETS_FILE")]
    secrets_file: String,

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
    /// Use "?" to list available devices.
    /// If omitted, uses the system default output device.
    #[arg(short, long, default_value = None, env = "PLEEZER_DEVICE")]
    device: Option<String>,

    /// Enable volume normalization
    ///
    /// Normalizes volume across tracks to provide consistent listening levels.
    #[arg(long, default_value_t = false, env = "PLEEZER_NORMALIZE_VOLUME")]
    normalize_volume: bool,

    /// Prevent other clients from taking over the connection
    ///
    /// By default, other clients can interrupt and take control of playback.
    #[arg(long, default_value_t = false, env = "PLEEZER_NO_INTERRUPTIONS")]
    no_interruptions: bool,

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

/// Initializes the logger facade.
///
/// The logging level is determined as follows, in order of precedence from
/// highest to lowest:
/// 1. Command line arguments
/// 2. `RUST_LOG` environment variable
/// 3. Hard-coded default
///
/// # Parameters
///
/// - `config`: a `&Args` with the command line arguments.
///
/// # Panics
///
/// Panics when a logger facade is already initialized.
fn init_logger(config: &Args) {
    let mut logger = env_logger::Builder::from_env(
        // Note: if you change the default logging level here, then you should
        // probably also change the verbosity levels below.
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    if config.quiet || config.verbose > 0 {
        let level = match config.verbose {
            0 => {
                // Quiet and verbose are mutually exclusive, and `verbose` is 0
                // by default. So this arm means: quiet mode.
                LevelFilter::Warn
            }
            1 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        };

        // Filter log messages of pleezer.
        logger.filter_module(module_path!(), level);
    }

    // Filter log messages of external crates.
    logger.filter_module("symphonia_bundle_flac", LevelFilter::Warn);
    logger.filter_module("symphonia_bundle_mp3", LevelFilter::Warn);
    logger.filter_module("symphonia_core", LevelFilter::Warn);
    logger.filter_module("symphonia_metadata", LevelFilter::Warn);

    logger.init();
}

/// Parse the secrets file into a `toml::Value`.
fn parse_secrets(secrets_file: impl AsRef<Path>) -> Result<toml::Value> {
    // Prevent out-of-memory condition: secrets file should be small.
    let attributes = fs::metadata(&secrets_file)?;
    let file_size = attributes.len();
    if file_size > 1024 {
        return Err(Error::out_of_range(
            "{secrets_file} too large: {file_size} bytes",
        ));
    }

    let contents = fs::read_to_string(&secrets_file)?;
    contents.parse::<toml::Value>().map_err(|e| {
        Error::invalid_argument(format!(
            "{} format invalid: {e}",
            secrets_file.as_ref().to_string_lossy()
        ))
    })
}

/// Main application loop.
///
/// # Parameters
///
/// - `args`: a `Args` with the command line arguments.
///
/// # Returns
///
/// - `Ok`: a `()` when the application exits successfully.
/// - `Err`: a `Box<dyn Error>` when an error occurs.
///
/// # Errors
///
/// This function returns `Err` when an error occurs. This could be due to the
/// user interrupting the application or an unrecoverable network error.
async fn run(args: Args) -> Result<()> {
    if args.device.as_ref().is_some_and(|device| device == "?") {
        // List available devices and exit.
        let devices = Player::enumerate_devices();
        if devices.is_empty() {
            return Err(Error::not_found("no audio output devices found"));
        }

        info!("available audio output devices:");
        for device in devices {
            info!("- {device}");
        }
        return Ok(());
    }

    if let Ok(proxy) = env::var("HTTPS_PROXY") {
        info!("using proxy: {proxy}");
    }

    let config = {
        // Get the credentials from the secrets file.
        info!("parsing secrets from {}", args.secrets_file);
        let secrets = parse_secrets(args.secrets_file)?;

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
                    .ok_or(Error::unauthenticated("email not found"))?;
                let password = secrets
                    .get("password")
                    .and_then(|password| password.as_str())
                    .ok_or(Error::unauthenticated("password not found"))?;

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

        let device_id = match machine_uid::get() {
            Ok(machine_id) => {
                let namespace = Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"deezer.com");
                Uuid::new_v5(&namespace, machine_id.as_bytes())
            }
            Err(e) => {
                warn!("could not get machine id, using random device id: {e}");
                Uuid::new_v4()
            }
        };
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
            "application name, version and/or language invalid (\"{app_name}\"; \"{app_version}\"; \"{app_lang}\")")
        ));
        }

        let os_name = match std::env::consts::OS {
            "macos" => "osx",
            other => other,
        };
        let os_version = sysinfo::System::os_version().unwrap_or_else(|| String::from("0"));
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
        let client_id = with_rng(|rng| rng.gen_range(100_000_000..=999_999_999));
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

            hook: args.hook,

            client_id,
            user_agent,

            credentials,
            bf_secret,

            eavesdrop: args.eavesdrop,
        }
    };

    let player = Player::new(&config, args.device.as_deref().unwrap_or_default()).await?;
    let mut client = remote::Client::new(&config, player)?;

    // Restart after sleeping some duration to prevent accidental denial of
    // service attacks on the Deezer infrastructure. Initially set the timer to
    // zero to immediately connect to the Deezer servers.
    let restart_timer = tokio::time::sleep(Duration::ZERO);
    tokio::pin!(restart_timer);

    // Main application loop. This restarts the new remote client when it gets
    // disconnected for whatever reason. This could be from a network failure
    // on either end or simply a disconnection from the user. In this case, the
    // session is refreshed with possibly new user data.
    loop {
        tokio::select! {
            // Prioritize shutdown signals.
            biased;

            // Handle shutdown signals.
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down gracefully");
                client.stop().await;
                break Ok(())
            }

            // Restart the client when it gets disconnected. The initial
            // connection happens immediately, because the timer elapses
            // immediately.
            result = client.start(), if restart_timer.is_elapsed() => {
                // Bail out if the error is a permission denied error. This
                // could be due to the user not being able to login.
                // Otherwise, try to recover from the error by restarting the
                // client.
                if let Err(e) = &result {
                    if e.kind == ErrorKind::PermissionDenied {
                        break result;
                    }

                    error!("{e}");
                }

                // Sleep with jitter to prevent thundering herds. Subsecond
                // precision further prevents that by spreading requests
                // when users are launching this from some crontab.
                let duration = Duration::from_millis(with_rng(|rng| rng.gen_range(5_000..6_000)));
                info!("restarting in {:.1}s", duration.as_secs_f32());
                restart_timer.as_mut().reset(tokio::time::Instant::now() + duration);
            }

            // Keep the timer running until the client is ready to restart.
            _ = &mut restart_timer, if !restart_timer.is_elapsed() => {}
        }
    }
}

/// Main entry point of the application.
///
/// This function initializes the logger facade, parses the command line
/// arguments, and starts the main application loop.
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
    let version = cmd.get_version().unwrap_or("UNKNOWN").to_string();
    let lang = String::from("en");

    info!("starting {name}/{version}; {BUILD_PROFILE}; {lang}");

    if let Err(e) = run(args).await {
        error!("{e}");
        process::exit(1);
    }
}
