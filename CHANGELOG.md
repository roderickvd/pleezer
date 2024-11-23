# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
and [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

## [Unreleased]

### Added
- [chore] Add Debian package metadata

### Changed
- [protocol] Parse JSON as 64-bit and truncate internally

### Deprecated

### Removed

### Fixed
- [config] Hexademical base does not correlate to key length

### Security

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

[Unreleased]: https://github.com/roderickvd/pleezer/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.2.0
[0.1.0]: https://github.com/roderickvd/pleezer/releases/tag/v0.1.0
