# pleezer

## ⚠️ IMPORTANT: Deezer Connect Deprecation Notice

**Deezer has officially deprecated Deezer Connect** as announced at https://en.deezercommunity.com/product-updates/say-goodbye-to-deezer-connect-80661. **It is unknown how long pleezer will continue to work.**

Currently, Deezer Connect functionality remains available on:
- **iOS**: When "Remote Control" is enabled under Deezer Lab settings
- **Android**: By using an older APK version

**Users should be aware that this functionality may stop working at any time** as Deezer continues to phase out Deezer Connect support.

---

**pleezer** turns your computer into a [Deezer Connect](https://support.deezer.com/hc/en-gb/articles/5449309457949-Deezer-Connect) streaming device that you can control from your phone, just like a Chromecast or smart speaker. Perfect for home audio setups, DIY projects, and custom integrations.

**Important:** A paid [Deezer subscription](https://deezer.com/offers) is required. Free accounts will not work with pleezer.

## Quick Start

### Using moOde Audio Player (Raspberry Pi)

The easiest way to use pleezer on a Raspberry Pi is through [moOde audio player](https://moodeaudio.org/):
1. Install moOde on your Raspberry Pi
2. Enable pleezer in moOde's user interface
3. Configure your Deezer account
4. Control from your phone!

### Manual Installation

1. Install pleezer:
   ```bash
   cargo install pleezer
   ```
2. Create a `secrets.toml` file with your Deezer login:
   ```toml
   email = "your-email@example.com"
   password = "your-password"
   ```
3. Run pleezer:
   ```bash
   pleezer
   ```
4. Control from your phone:
   - Open the Deezer app
   - Tap the speaker icon (bottom-left)
   - Select "Deezer Connect"
   - Choose your pleezer device
   - Start playing!

Need help? Check out [Troubleshooting](#troubleshooting) or join our [Discussions](https://github.com/roderickvd/pleezer/discussions).

## Important Disclaimer

**pleezer** is an independent project and is not affiliated with, endorsed by, or created by Deezer. It is developed to provide a streaming player that is fully compatible with the Deezer Connect protocol.



**pleezer** **does not and will not support** saving or extracting music files for offline use. This project:
- Respects artists' rights and opposes piracy
- Only supports legitimate streaming through Deezer Connect
- Properly reports playback for artist monetization
- Does not include decryption keys in the code

## Key Features

- Stream music in formats from MP3 to lossless FLAC (depending on your subscription)
- Access your full Deezer library: songs, podcasts, radio, mixes, and Flow
- High-quality audio processing:
  * High-quality dithering with Shibata noise shaping
  * Volume-aware dither scaling
  * Smart volume normalization
- Connect to standard audio outputs, or use JACK (Linux) or ASIO (Windows)
- Automate with hook scripts and external controls
- Run reliably with stateless operation and proper signal handling

## Basic Usage

The audio quality setting in your Deezer app controls the streaming quality to pleezer:

1. In the Deezer mobile app, go to Settings > Audio
2. Under "Google Cast", select your preferred quality:
   - Basic Quality (64 kbps MP3)
   - Standard Quality (128 kbps MP3)
   - High Quality (320 kbps MP3)
   - High Fidelity (FLAC, up to 1411 kbps)

**Notes:**
- Radio streams use their best available quality up to your selected setting
- Podcasts always stream in their original quality
- Your subscription level determines available quality options

When using Deezer Connect, your phone's battery may drain faster than usual. This is normal - the Deezer app needs to maintain constant communication with pleezer for remote control.

## Common Configuration

### Audio Output

By default, pleezer uses your system's default audio output. To use a specific device:

1. List available devices:
   ```bash
   pleezer -d "?"
   ```

2. Select a device:
   ```bash
   pleezer -d "device-name"
   ```

Common examples:
```bash
pleezer -d "Built-in Output"              # Use built-in audio
pleezer -d "USB DAC"                      # Use USB audio device
pleezer -d "JACK|cpal_audio_out"          # Connect to JACK (Linux)
pleezer -d "ASIO|USB Audio Interface"     # Use ASIO device (Windows)
```

### Volume Control

Set initial volume level (0-100):
```bash
pleezer --initial-volume 50  # Start at 50% volume
```

Enable volume normalization:
```bash
pleezer --normalize-volume
```

### Custom Name

Change how pleezer appears in the Deezer app:
```bash
pleezer --name "Living Room"
```

## Authentication

### Using Email and Password (Recommended)

Create a `secrets.toml` file containing:
```toml
email = "your-email@example.com"
password = "your-password"
```

By default, pleezer looks for this file in the current directory. Use `-s` to specify a different location:
```bash
pleezer -s /path/to/secrets.toml
```

### Using ARL (Alternative)

If you prefer not to store your password, you can use a temporary Authentication Reference Link (ARL):

1. Visit [Deezer login callback](https://www.deezer.com/desktop/login/electron/callback)
2. Log in to your account
3. Copy the ARL from the button (the part after `deezer://autolog/`)
4. Create a `secrets.toml` file containing:
   ```toml
   arl = "your-arl"
   ```

**Note:** ARLs expire periodically. Email/password authentication is more reliable for long-term use.

## Hook Scripts

Hook scripts let you automate actions when events occur (like tracks changing or playback starting). Use the `--hook` option to specify your script:

```bash
pleezer --hook /path/to/script.sh
```

Your script receives event information through environment variables. Example script:

```bash
#!/bin/bash
case "$EVENT" in
"track_changed")
    # Safely print track info by escaping special characters
    echo "Now playing: $(printf %q "$TITLE") by $(printf %q "$ARTIST")"

    # Run longer operations in background to avoid delays
    update_home_automation "$(printf %q "$TITLE")" "$(printf %q "$ARTIST")" &
    ;;
"connected")
    echo "Connected as: $(printf %q "$USER_NAME")"
    ;;
esac
```

**Important:**
- Keep scripts quick and simple
- Run time-consuming operations in the background
- Always use `printf %q` to safely escape variables

### Available Events

#### Playback Events

`playing` - When playback starts
- `TRACK_ID`: ID of the playing track

`paused` - When playback pauses
- No additional variables

`track_changed` - When the track changes
- `TRACK_TYPE`: "song", "episode", or "livestream"
- `TRACK_ID`: Content ID
- `TITLE`: Track/episode title (not set for radio)
- `ARTIST`: Artist/podcast/station name
- `ALBUM_TITLE`: Album name (songs only)
- `COVER_ID`: Artwork ID
- `DURATION`: Length in seconds (not set for radio)
- `FORMAT`: Input format and bitrate (e.g., "MP3 320K", "FLAC 1.234M")
- `DECODER`: Output format (e.g., "PCM 16 bit 44.1 kHz, Stereo")

#### Connection Events

`connected` - When a controller connects
- `USER_ID`: Your Deezer user ID
- `USER_NAME`: Your Deezer username

`disconnected` - When a controller disconnects
- No additional variables

### Cover Art URLs

Use the `COVER_ID` to construct artwork URLs:

For songs and radio:
```
https://cdn-images.dzcdn.net/images/cover/{cover_id}/{size}x{size}.{format}
```

For podcasts:
```
https://cdn-images.dzcdn.net/images/talk/{cover_id}/{size}x{size}.{format}
```

Where:
- `{size}`: Image size in pixels (up to 1920)
- `{format}`: `jpg` (smaller) or `png` (higher quality)

Example: `500x500.jpg` is Deezer's default size

## Advanced Configuration

### Audio Device Selection

The `-d` option accepts detailed device specifications:
```
[<host>][|<device>][|<sample rate>][|<sample format>]
```

All parts are optional and case-insensitive:
- Skip any part using `|`
- Omit trailing parts entirely

Sample formats:
- `i16`: 16-bit integer (most compatible)
- `i32`: 32-bit integer (better for volume control)
- `f32`: 32-bit float (best quality)

Examples by platform:

Linux (ALSA):
```bash
pleezer -d "ALSA|Yggdrasil+"                # Named device with default configuration
pleezer -d "ALSA|Yggdrasil+|44100|i32"      # Named device with sampling rate and format
```

**Using ALSA Virtual Devices:**
Virtual devices like `_audioout` or `camilladsp` are not directly enumerable. To use virtual devices, configure ALSA to route the default device to your virtual device by adding a configuration like this to one of:
- `~/.asoundrc` (user-specific)
- `/etc/asound.conf` (system-wide)
- `/etc/alsa/conf.d/default.conf` (system-wide, recommended)

```
pcm.!default {
    type plug
    slave.pcm "_audioout"
}

ctl.!default {
    type hw
    card 0
}
```

Then run pleezer with either no `-d` option or `-d "ALSA|default"`:
```bash
pleezer                                     # Uses system default
pleezer -d "ALSA|default"                   # Explicitly use default
```

Linux (JACK) - requires `--features jack`:
```bash
pleezer -d "JACK|pleezer_out"               # Custom client name
```

macOS:
```bash
pleezer -d "CoreAudio|DAC|44100|f32"        # DAC with format
pleezer -d "|External Speakers"             # Just device name
```

Windows (WASAPI):
```bash
pleezer -d "WASAPI|Speakers|44100"          # With sample rate
pleezer -d "||48000"                        # Just sample rate
```

Windows (ASIO) - requires `--features asio`:
```bash
pleezer -d "ASIO|USB Interface"             # ASIO device
```

**Notes:**
- Music plays at 44.1 kHz
- Podcasts/radio may use other rates (e.g., 48 kHz)
- Resampling happens automatically when needed
- 32-bit formats (i32/f32) recommended with volume normalization
- Advanced: While device enumeration shows only common configurations (44.1/48 kHz, I16/I32/F32), other sample rates (e.g., 96 kHz) and formats (e.g., U16) are supported when explicitly specified in the device string

### Audio Processing

#### Volume Normalization

Enable volume normalization for consistent levels:
```bash
pleezer --normalize-volume
```

The normalizer provides intelligent gain adjustment to reach Deezer's target level (-15 dB LUFS):
- For negative gain (loud tracks): Simple attenuation of average signal level
- For positive gain (quiet tracks): Dynamic limiting to prevent clipping while preserving dynamics

This approach ensures:
- No clipping when boosting quiet tracks
- No unnecessary processing on tracks that only need attenuation
- Maximum dynamic range preservation

#### Loudness Compensation

Enable psychoacoustic loudness compensation:
```bash
pleezer --loudness
```

Compensates for how human hearing perceives different frequencies, especially at lower volumes:
- Applies frequency-dependent gain based on ISO 226:2013 research
- Maintains tonal balance across volume levels
- Particularly beneficial for quiet listening
- Automatically scales with volume setting

The compensation effect:
- Strong at low volumes where hearing sensitivity varies most
- Gradually reduces as volume increases

#### Dithering

pleezer improves audio quality through:
- High-quality triangular (TPDF) dithering
- Volume-aware dither scaling to preserve dynamic range
- Automatic adjustment based on content and playback settings

The dithering process:
- Applies when requantizing audio for your DAC
- Adapts to volume changes to maintain quality

Configure dithering based on your DAC's measured performance:
```bash
# Example for DAC with THD+N of -118 dB:
pleezer --dither-bits 19.3

# Disable dithering entirely:
pleezer --dither-bits 0
```

Calculate optimal dither bits from DAC specifications:
- For THD+N in dB: (-dB - 1.76) / 6.02
  Example: THD+N of -118 dB → 19.3 bits
- For THD+N as percentage: (-20 * log10(percentage) - 1.76) / 6.02
  Example: THD+N of 0.0002% → 18.6 bits
- Use 0 to disable dithering

#### Noise Shaping

pleezer uses psychoacoustic noise shaping to optimize audio quality:
- Pushes quantization noise into less audible frequencies
- Uses modern Shibata coefficients for optimal noise distribution
- Provides several noise shaping levels:
  * Level 0: No shaping (plain TPDF dither) - safest, recommended for podcasts
  * Level 1: Very mild shaping (~5 dB ultrasonic rise)
  * Level 2: Mild shaping (~8 dB rise) - recommended default for most music
  * Level 3: Moderate shaping (~12 dB rise) - can benefit classical/jazz/ambient
  * Level 4-7: Not recommended - excessive ultrasonic energy that may stress audio equipment

Configure noise shaping:
```bash
# Use noise shaping level 2 (recommended default)
pleezer --noise-shaping 2

# Use level 0 for podcasts (safest)
pleezer --noise-shaping 0
```

Recommendations by content type:
- Podcasts: Level 0 (pure dither, no shaping)
- Most music (rock, pop, metal, EDM): Level 1-2
- Classical, jazz, ambient: Level 2-3
- Vintage/lo-fi material: Level 0 or 1

### Memory Usage

Control RAM usage for audio buffering:
```bash
pleezer --max-ram 64  # Use up to 64MB RAM
```

Approximate sizing:
- MP3 (320 kbps): ~15MB per 5-minute track
- FLAC: ~30-50MB per 5-minute track

Double these amounts to handle current and preloaded tracks:
- `--max-ram 100` for MP3
- `--max-ram 200` for FLAC

If a track exceeds the limit or `--max-ram` isn't set, temporary files are used instead.

### Connection Control

Prevent other devices from taking control:
```bash
pleezer --no-interruptions
```

Specify network interface:
```bash
pleezer --bind 192.168.1.2     # Specific IPv4 interface
pleezer --bind ::1             # IPv6 loopback
```

### Environment Variables

All options can be set with environment variables using the prefix `PLEEZER_` and SCREAMING_SNAKE_CASE:

```bash
# Set in environment
export PLEEZER_NAME="Living Room"
export PLEEZER_NO_INTERRUPTIONS=true
export PLEEZER_INITIAL_VOLUME=50

# Override with arguments
pleezer --name "Kitchen"  # Takes precedence
```

### Proxy Support

Set proxy for all connections using the `HTTPS_PROXY` environment variable:

```bash
# Linux/macOS
export HTTPS_PROXY="http://proxy.example.com:8080"    # HTTP proxy
export HTTPS_PROXY="https://proxy.example.com:8080"   # HTTPS proxy

# Windows (Command Prompt)
set HTTPS_PROXY=https://proxy.example.com:8080

# Windows (PowerShell)
$env:HTTPS_PROXY="https://proxy.example.com:8080"
```

## Troubleshooting

### Common Issues

#### Connection Problems

**Device not showing in Deezer app**
- Verify you have a paid Deezer subscription
- Check that pleezer and your phone use the same Deezer account
- Ensure both devices are on the same network
- Try restarting the Deezer app

**Controls not responding**
- Disconnect and reconnect in the Deezer app
- If problem persists, force-quit and restart the Deezer app

#### Audio Issues

**Maximum volume on connect**
- Use `--initial-volume` to set a lower starting level
  ```bash
  pleezer --initial-volume 50
  ```

**Volume inconsistent between tracks**
- Enable volume normalization
  ```bash
  pleezer --normalize-volume
  ```
- Note: Not all tracks have normalization data

**Audio stops after device change**
- pleezer needs to be restarted if output device becomes unavailable
- Working on automatic reconnection for future versions

#### Known Limitations

- Cannot control from desktop apps or web player (Deezer Connect limitation)
- Favorites list may not work (known Deezer app issue)
- Some Pi Zero 2W users may need to disable IPv6 DNS lookups:
  ```
  # Add to /etc/resolv.conf
  options no-aaaa
  ```

### Debug Options

For troubleshooting, enable debug logging:
```bash
pleezer -v     # Debug logging
pleezer -vv    # Trace logging (very detailed)
```

Suppress non-essential output:
```bash
pleezer -q     # Only show warnings and errors
```

Monitor protocol messages (development):
```bash
pleezer --eavesdrop -vv
```

## Building pleezer

**pleezer** is supported on Linux and macOS with full compatibility. Windows support is tier two, meaning it is not fully tested and complete compatibility is not guaranteed. Contributions to enhance Windows support are welcome.

### System Requirements

#### Linux
```bash
# Debian/Ubuntu
sudo apt-get update
sudo apt-get install build-essential libasound2-dev pkgconf

# Fedora
sudo dnf groupinstall 'Development Tools'
sudo dnf install alsa-lib-devel
```

#### macOS
```bash
xcode-select --install
```

#### Windows
- Install [Visual Studio](https://visualstudio.microsoft.com/) with C++ support

### Installing Rust

All platforms need Rust installed. Visit [rustup.rs](https://rustup.rs/) and follow the installation instructions for your system.

### Installation Methods

#### From crates.io (Stable)
```bash
cargo install pleezer
```

#### From Source (Development)
```bash
git clone https://github.com/roderickvd/pleezer.git
cd pleezer
cargo build --release
cargo install --path .  # Optional: system-wide install
```

### Optional Features

#### JACK Support (Linux)
```bash
# Debian/Ubuntu
sudo apt-get install libjack-dev

# Fedora
sudo dnf install jack-audio-connection-kit-devel

# Build with JACK support
cargo build --features jack
```

#### ASIO Support (Windows)
- Install Steinberg ASIO SDK
- Configure per [CPAL documentation](https://docs.rs/crate/cpal/latest)
- Build with ASIO support:
  ```bash
  cargo build --features asio
  ```

## Pre-Built Installations

**pleezer** is available as part of these distributions:

- [moOde audio player](https://moodeaudio.org/): A complete Raspberry Pi-based audiophile music player distribution

If you maintain a project, product, or distribution that includes **pleezer**, feel free to submit a pull request to add it to this list.

## Related Projects

These projects have influenced **pleezer**:

- [deezer-linux](https://github.com/aunetx/deezer-linux): An unofficial Linux port of the native Deezer Windows application, providing offline listening capabilities
- [librespot](https://github.com/librespot-org/librespot): An open-source client library for Spotify with support for Spotify Connect
- [lms-deezer](https://github.com/philippe44/lms-deezer): A plugin for Logitech Media Server to stream music from Deezer

## Acknowledgements
This project uses:
- Shibata noise shaping coefficients from [SSRC](https://github.com/shibatch/SSRC) by Naoki Shibata, licensed under LGPL-2.1
- See [ATTRIBUTION.md](https://github.com/roderickvd/pleezer/blob/main/ATTRIBUTION.md) for details on third-party components

## Legal Information

### License

**pleezer** uses the [Sustainable Use License](https://github.com/roderickvd/pleezer/blob/main/LICENSE.md), which promotes fair use and sustainable open-source development.

#### Personal/Non-Commercial Use
- Free to use, modify, and distribute
- Can integrate into other free software/hardware
- Must maintain free access for users

#### Commercial Use
Requires a commercial license when:
- Including in paid software/hardware
- Using in products with paid features
- Distributing as part of a paid service

This helps ensure fair compensation for development work and continued project maintenance.

### Using pleezer with Deezer

When using pleezer, you must follow [Deezer's Terms of Service](https://www.deezer.com/legal/cgu):
- Use only for personal/family streaming
- Maintain a valid paid subscription
- Don't extract or save content offline
- Allow proper playback reporting for artists

## Security

### Reporting Issues

- **Security Vulnerabilities**: Email the author directly
- **General Issues**: Use [GitHub Issues](https://github.com/roderickvd/pleezer/issues)
- See [Security Policy](https://github.com/roderickvd/pleezer/blob/main/SECURITY.md) for details

### Secrets File Safety

Keep your `secrets.toml` file secure:
- Store in a private location
- Don't share or commit to repositories
- Contains sensitive account access information

## Support and Contact

### Getting Help

1. Check [Troubleshooting](#troubleshooting) section
2. Search [GitHub Issues](https://github.com/roderickvd/pleezer/issues)
3. Join [GitHub Discussions](https://github.com/roderickvd/pleezer/discussions)

### Contributing

Contributions welcome! See [Contributing Guidelines](https://github.com/roderickvd/pleezer/blob/main/CONTRIBUTING.md) for:
- Submitting issues
- Creating pull requests
- Code standards
- Development setup

### Supporting Development

If you find pleezer useful, consider:
- Contributing code or documentation
- Reporting issues and testing fixes
- Supporting through [GitHub Sponsors](https://github.com/sponsors/roderickvd)

### Contact

- **Security Issues/Commercial Licensing**: Email author directly
- **Bug Reports**: [GitHub Issues](https://github.com/roderickvd/pleezer/issues)
- **General Discussion**: [GitHub Discussions](https://github.com/roderickvd/pleezer/discussions)
- Please don't use email for general support requests
