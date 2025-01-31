# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
and [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

## [Unreleased]

### Added
- [http] Support binding outgoing connections to specific network interfaces

### Fixed
- [http] Fix connection timeouts in dual-stack network environments
- [player] Fix audio device being held before playback starts

## [v0.11.1] - 2025-01-27

### Changed
- [normalize] Remove `ExactSizeIterator` implementation as total samples can't be determined exactly
- [remote] Improve session renewal reliability with proper cookie expiration handling

### Fixed
- [remote] Fix token refresh errors not triggering client restart during connection
- [remote] Fix malformed `Cookie` header in websocket connection

## [v0.11.0] - 2025-01-26

### Changed
- [codec] Split `frame_duration` into `max_frame_length` and `max_frame_duration`
- [decoder] Better error handling following Symphonia's recommendations
- [decoder] Always use accurate seeking mode for reliable position reporting
- [decoder] Fix logical error in `size_hint()` lower bound calculation
- [decoder] Remove `ExactSizeIterator` implementation as total samples can't be determined exactly
- [http] Simplified HTTP client response handling and content type management
- [http] Added status code checking in HTTP client responses
- [http] Increase read timeout to 5 seconds to accommodate slower devices
- [player] Improve seek logging with more detailed timestamps and progress information
- [remote] Improve network timeout handling and error messages

### Fixed
- [decoder] Fix calculation of total number of samples for size hint
- [gateway] Fix user token persistence by handling JWT token renewal

### Removed
- [error] Remove remaining Rodio decoder leftovers in favor of direct Symphonia usage

## [v0.10.0] - 2025-01-19

### Added
- [audio_file] Add 32 KiB buffering to all downloads for lower CPU usage
- [track] Report audio parameters through `DECODER` environment variable in hook scripts

### Changed
- [decrypt] Replace `StorageProvider` bound with `ReadSeek` for better abstraction
- [docs] Restructure installation section to acknowledge pre-packaged availability
- [player] Remove extra -1 dB headroom for lossy tracks as it's handled by the limiter threshold

### Fixed
- [normalize] Fix edge-case imaging in limiter by removing incorrect single-channel optimization

## [0.9.1] - 2025-01-18

### Changed
- [normalize] Improve limiter to handle multichannel audio while preserving imaging

## [0.9.0] - 2025-01-18

### Added
- [audio_file] Add unified `AudioFile` abstraction for audio stream handling
- [decoder] New Symphonia-based audio decoder for improved performance and quality:
  - Higher audio quality (`f32` processing instead of `i16`)
  - More robust AAC support in both ADTS and MP4 formats
  - WAV support (for some podcasts)
  - Faster seeking in FLAC and MP3 files
  - Faster decoder initialization
  - Lower memory pressure
- [normalize] Add professional-grade feedforward limiter for volume normalization
- [player] Use ReplayGain metadata as fallback for volume normalization when Deezer gain information is unavailable
- [util] Add audio gain conversion utilities for volume normalization calculations

### Changed
- [decrypt] Add explicit `BufRead` implementation to standardize buffering behavior
- [decrypt] Improve buffer management performance and efficiency
- [docs] Remove incorrect mention of "Hi-Res" audio quality
- [player] Default to mono audio for podcasts to prevent garbled sound when channel count is missing
- [track] Return `AudioFile` instead of raw download stream

### Fixed
- [docs] Update artwork URLs to use correct CDN paths for different content types
- [remote] Improve network resilience by automatically reconnecting after connection errors
- [track] Correct bitrate calculation for user-uploaded MP3s by excluding ID3 tags and album art
- [track] Cap reported bitrates to codec maximums (320 kbps for MP3, 1411 kbps for FLAC, etc.)

## [0.8.1] - 2025-01-11

### Added
- [docs] Add instructions for configuring audio quality through Deezer app settings

### Changed
- [decrypt] Remove redundant `'static` lifetime bounds from `StorageProvider` trait

### Fixed
- [main] Reduce default logging verbosity for audio codecs to ERROR level
- [track] Fix bitrate calculation for podcasts and variable quality streams

## [0.8.0] - 2025-01-05

### Added
- [main] Support for SIGHUP to reload configuration
- [remote] Add audio format and bitrate to `track_changed` event
- [signal] New module for unified signal handling across platforms
- [tests] Add anonymized API response examples
- [track] Support for podcast episodes with external streaming
- [track] Support for radio livestreams with multiple quality options and codecs
- [track] Support for fallback tracks when primary version is unavailable

### Changed
- [docs] Enhanced documentation for signal handling and lifecycle management
- [main] Improved signal handling and graceful shutdown
- [remote] Remove automatic shell escaping from hook script variables
- [remote] Improve error handling and ignore progress updates for livestreams
- [remote] Renamed `ALBUM_COVER` to `COVER_ID` in the `track_changed` event
- [track] Renamed `album_cover` to `cover_id` for consistency

### Fixed
- [player] Improve seek behavior by limiting to buffered data

## [0.7.0] - 2024-12-28

### Added
- [docs] Add anonymized API response fixtures as reference documentation
- [gateway] Check for Free accounts and prevent connecting due to audio ads limitation

### Changed
- [build] Switch from exclude to include for more precise package contents
- [gateway] More descriptive error messages for subscription-related issues
- [protocol] Add `ads_audio` field to user options structure
- [protocol] Centralize JSON response parsing and logging
- [protocol] Make duration parsing more flexible to handle non-standard time formats
- [protocol] Make track duration handling more robust for missing or invalid metadata

### Fixed
- [player] Prevent audio popping when changing tracks or stopping playback by adding volume ramping

## [0.6.2] - 2024-12-19

### Changed
- [docs] Improve documentation coverage
- [remote] Configure websocket message size limits to prevent memory exhaustion

### Fixed
- [remote] Prevent duplicate remotes appearing in older Deezer apps
- [remote] Initial volume not being set when controller reconnects
- [track] Infinite loop loading track that is not available yet or anymore

## [0.6.1] - 2024-12-13

### Added
- [docs] Add documentation link for docs.rs in package metadata

### Changed
- [build] Enable thin LTO and single codegen unit by default for better runtime performance

### Fixed
- [player] Fix disconnection when skipping to next track before current track finishes downloading

## [0.6.0] - 2024-12-12

### Added
- [docs] Instruct docs.rs to document all features
- [docs] Document battery usage with Deezer Connect
- [main] Print Git commit hash and date if available

### Changed
- [error] Represent gRPC status codes as `u32`
- [remote] Improved connection robustness by removing offer ID validation
- [remote] Centralize close message handling

### Fixed
- [docs] Fix Rustdoc lints and warnings
- [remote] Restart client on user token expiration
- [remote] Fix event handling after client restart

## [0.5.0] - 2024-12-09

### Added
- [player] Support JACK and ASIO audio backends
- [player] Queue reordering with position tracking
- [remote] Queue shuffle support with state synchronization
- [remote] Initial volume setting that remains active until client takes control below maximum

### Fixed
- [docs] Fix Rustdoc linking to error module in documentation

## [0.4.0] - 2024-12-02

### Added
- [docs] Comprehensive documentation for all public APIs and internals
- [docs] Recommendation to use 32-bit output formats for better audio precision
- [error] Add `downcast()` method to access underlying error types
- [player] Explicit audio device lifecycle with `start()`, `stop()` and `is_started()`
- [uuid] `uuid` module providing a fast UUID v4 generator

### Changed
- [docs] Clarify that Deezer Connect control only works from mobile devices
- [gateway] Use UNIX epoch instead of current time for expired token state
- [main] Use kernel instead of distribution version on Linux systems
- [player] Scale volume logarithmically with 60 dB dynamic range
- [player] Only show output devices that support stereo 44.1/48 kHz in I16/I32/F32 formats
- [remote] Start/stop audio device on controller connect/disconnect
- [remote] Improve connection handshake ordering and timeout handling

### Fixed
- [protocol] Use epsilon comparison for `Percentage` equality checks
- [player] Prevent from acquiring output device before playback starts
- [player] Default device was not enumerated on Alsa
- [remote] Improve queue refresh handling
- [remote] Fix race condition in controller connection setup
- [tokens] Fix token expiration check

### Removed
- [docs] Remove unnecessary Homebrew installation instructions

## [0.3.0] - 2024-11-28

### Added
- [chore] Add Debian package metadata
- [docs] Add ALSA device examples to the README
- [main] Add `--device-type` option to control device identification

### Changed
- [build] Document release profile optimization options
- [docs] Improve Rustdoc of `protos`, `channel` and `contents` modules
- [gateway] Improve error logging for response parsing failures
- [main] Clearer log messages for secrets and logins
- [protocol] Parse JSON as 64-bit and truncate internally

### Fixed
- [config] Hexademical base does not correlate to key length
- [gateway] Parse user data without all fields present
- [gateway] Incorrect user token expiry
- [player] Use pipe separator in device specs for ALSA compatibility
- [player] Clean up audio device enumeration output
- [player] Playback progress not updating correctly after third track
- [player] Delay reporting playback progress after a track change
- [repo] Fix pull request template format
- [remote] Trigger connected and disconnected events

## [0.2.0] - 2024-11-23

### Added
- [main] Support for configuring all command-line options via environment variables with `PLEEZER_` prefix
- [proxy] HTTPS proxy support via the `HTTPS_PROXY` environment variable
- [remote] Websocket monitoring mode for Deezer Connect protocol analysis
- [remote] Hook script support to execute commands on playback and connection events

### Changed
- [docs] Enhanced documentation clarity and consistency across all policy documents
- [main] Improved command-line argument descriptions and examples
- [main] Made command-line parsing dependency (`clap`) optional and binary-only
- [player] Optimized track skipping using `HashSet` for better performance
- [track] Get the media URL programmatically instead of hardcoding it

### Fixed
- [protocol] Correctly handle `connect` messages missing the `offer_id` field

### Security
- [arl] Prevent ARL token exposure in debug logs

## [0.1.0] - 2024-11-20

Initial release of pleezer, a headless streaming player for the Deezer Connect protocol.

### Added
- High-quality audio streaming (basic, HQ, or lossless) based on subscription
- Gapless playback for seamless transitions
- Flow and mixes support
- Playback reporting for artist monetization
- User MP3 playback support
- Deezer authentication via email/password or ARL token
- Volume normalization with clipping prevention
- Configurable audio output device selection
- Debug and trace logging capabilities
- Command-line interface with various configuration options

[Unreleased]: https://github.com/roderickvd/pleezer/compare/v0.11.1...HEAD
[0.11.1]: https://github.com/roderickvd/pleezer/releases/tag/v0.11.1
[0.11.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.11.0
[0.10.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.10.0
[0.9.1]: https://github.com/roderickvd/pleezer/releases/tag/v0.9.1
[0.9.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.9.0
[0.8.1]: https://github.com/roderickvd/pleezer/releases/tag/v0.8.1
[0.8.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.8.0
[0.7.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.7.0
[0.6.2]: https://github.com/roderickvd/pleezer/releases/tag/v0.6.2
[0.6.1]: https://github.com/roderickvd/pleezer/releases/tag/v0.6.1
[0.6.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.6.0
[0.5.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.5.0
[0.4.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.4.0
[0.3.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.3.0
[0.2.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.2.0
[0.1.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.1.0
