# pleezer

**pleezer** is an open-source, headless streaming player built around the [Deezer Connect](https://support.deezer.com/hc/en-gb/articles/5449309457949-Deezer-Connect) protocol. "Headless" means it runs without a graphical interface, making it ideal for DIY setups, server-based systems, or custom integrations where flexibility is key.

## Project Status

**pleezer** is a new project and is actively being developed. We welcome contributions and feedback from the community. For more details, see the [Contributing](#contributing) section.

## Table of Contents

- [Features](#features)
- [Installation](#installation)
- [Usage](#usage)
  - [Command-Line Arguments](#command-line-arguments)
  - [Environment Variables](#environment-variables)
  - [Proxy Configuration](#proxy-configuration)
  - [Hook Scripts](#hook-scripts)
  - [Stateless Configuration](#stateless-configuration)
  - [Configuring the Secrets File](#configuring-the-secrets-file)
- [Troubleshooting](#troubleshooting)
- [Setting Up Your Build Environment](#setting-up-your-build-environment)
- [Contributing](#contributing)
- [Support My Work](#support-my-work)
- [Related Projects](#related-projects)
- [License](#license)
- [Important Information](#important-information)
  - [Important Disclaimer](#important-disclaimer)
  - [Deezer Terms of Service](#deezer-terms-of-service)
  - [Security](#security)
- [Contacting the Author](#contacting-the-author)

## Features

### Supported Features

- **High-Quality Audio**: Stream in basic, HQ, or lossless formats depending on your [Deezer subscription](http://www.deezer.com/offers).
- **Gapless Playback**: Enjoy seamless transitions between tracks.
- **Flow and Mixes**: Access personalized playlists and mixes tailored to your preferences.
- **Playback Reporting**: Contribute to accurate artist monetization metrics.
- **User MP3 Support**: Play your uploaded MP3 files alongside streamed content.
- **Authentication Options**: Log in via Deezer email/password or ARL token.
- **Volume Normalization**: Maintain consistent volume across all tracks while preventing clipping or distortion.
- **Proxy Support**: Connect through HTTPS proxies using system environment variables.

### Planned Features

- Shuffle support
- Live radio and podcast integration
- Advanced audio outputs: ASIO, JACK
- Device registration

## Installation

Before installing **pleezer**, ensure your system has the necessary build environment set up. Refer to the [Setting Up Your Build Environment](#setting-up-your-build-environment) section for instructions.

**pleezer** can be installed in one of two ways:

1. **Install the Latest Stable Version**

   You can install the latest stable release of **pleezer** directly from [crates.io](https://crates.io/crates/pleezer) using Cargo:

   ```bash
   cargo install pleezer
   ```

   This command downloads, compiles, and installs the latest release version of **pleezer**. The binary will be placed in `~/.cargo/bin/` on Unix-like systems or `C:\Users\<YourUsername>\.cargo\bin\` on Windows.

2. **Build the Latest Development Version**

   If you want the latest development version, follow these steps:

   1. Clone the repository:
      ```bash
      git clone https://github.com/your-repo/pleezer.git
      cd pleezer
      ```

   2. Build the project:
      ```bash
      cargo build --release
      ```

      This command compiles the project and produces the binary in the `target/release/` directory.

   3. (Optional) Install the built version system-wide:
      ```bash
      cargo install --path .
      ```

      This installs the binary into `~/.cargo/bin/` on Unix-like systems or `C:\Users\<YourUsername>\.cargo\bin\` on Windows.

## Usage

Once **pleezer** is launched, it will wait for a connection from another Deezer client. Here's how to connect and use **pleezer** with your mobile device or other Deezer clients:

1. **Open** the Deezer app on your mobile device or another Deezer client.
2. **Tap** the loudspeaker icon, usually found in the bottom-left corner, to access the Audio Output section.
3. **Select** **Deezer Connect** to view a list of available devices.
4. **Choose** the device named with either the name you specified using the `-n` option or the default system hostname.

Your music will start playing on the selected device.

**Note:** To discover and connect to the **pleezer** device, ensure it is connected with the same Deezer account and that **pleezer** is online.

### Command-Line Arguments

- `-n` or `--name`: Set the player's name as it appears to Deezer clients. By default, it uses the system hostname. Example:
    ```bash
    pleezer --name "My Deezer Player"
    ```

- `-d` or `--device`: Select the output device. Use `?` to list available devices. If omitted, the system default output device is used. Examples:
    ```bash
    # List available devices
    pleezer -d "?"

    # Use a specific device, formatted as:
    # "[<host>][|<device>][|<sample rate>][|<sample format>]" (case-insensitive)
    #
    # All fields are optional:
    # - If you don't specify a host, it will use the system default host.
    # - If you don't specify a device, it will use the host default device.
    # - If you don't specify a sample rate, it will use the device default sample rate.
    # - If you don't specify a sample format, it will use the device default sample format.
    pleezer -d "CoreAudio"
    pleezer -d "CoreAudio|Yggdrasil+"
    pleezer -d "CoreAudio|Yggdrasil+|44100"
    pleezer -d "CoreAudio|Yggdrasil+|44100|f32"

    # Some more advanced examples, showing you can omit fields as long as you include the pipes:
    pleezer -d "|yggdrasil+"    # The Yggdrasil+ device (case-insensitive)
    pleezer -d "||44100"        # The first device to support 44100 Hz
    ```

    Deezer streams audio at 44100 Hz exclusively. Sampling rates other than
    44100 Hz are not recommended and provided for compatibility only.
    Resampling is done using linear interpolation.

- `--no-interruptions`: Prevent other clients from taking over the connection after **pleezer** has connected. By default, interruptions are allowed. Example:
    ```bash
    pleezer --no-interruptions
    ```

- `--hook`: Specify a script to execute when events occur (see [Hook Scripts](#hook-scripts) for details). Example:
    ```bash
    pleezer --hook /path/to/script.sh
    ```
    **Note:** The script must be executable and have a shebang line.

- `-q` or `--quiet`: Suppresses all output except warnings and errors. Example:
    ```bash
    pleezer -q
    ```

- `-v` or `--verbose`: Enables debug logging. Use `-vv` for trace logging. The `--quiet` and `--verbose` options are mutually exclusive. Examples:
    ```bash
    pleezer -v    # Debug logging
    pleezer -vv   # Trace logging
    ```

- `--eavesdrop`: Listen to the Deezer Connect websocket without participating. This is useful for development purposes and requires verbose or probably trace logging (`-v` or `-vv`). Example:
    ```bash
    pleezer --eavesdrop -vv
    ```

    **Note:** This option provides only partial insight into client communications. While some messages are echoed across all websockets belonging to a user, most messages are sent on separate websockets specific to each client. For complete traffic analysis, monitoring of all websockets would be required.

- `-s` or `--secrets`: Specify the secrets configuration file. Defaults to `secrets.toml`. Example:
    ```bash
    pleezer -s /path/to/secrets.toml
    ```

- `-h` or `--help`: Display help information about command-line options and exit. Example:
    ```bash
    pleezer -h
    ```

- `--version`: Show **pleezer** version and build information, then exit. Example:
    ```bash
    pleezer --version
    ```

### Environment Variables

All command-line options can be set using environment variables by prefixing `PLEEZER_` to the option name in SCREAMING_SNAKE_CASE. For example:

```bash
# Using environment variables
export PLEEZER_NAME="Living Room"
export PLEEZER_NO_INTERRUPTIONS=true

# Command-line arguments override environment variables
pleezer --name "Kitchen"  # Will use "Kitchen" instead
```

Command-line arguments take precedence over environment variables if both are set.

### Proxy Configuration

**pleezer** supports proxy connections through the `HTTPS_PROXY` environment variable. The value must include either the `http://` or `https://` schema prefix. HTTPS can be tunneled over either HTTP or HTTPS proxies.

Examples:

```bash
# Linux/macOS
export HTTPS_PROXY="http://proxy.example.com:8080"   # HTTPS over HTTP proxy
export HTTPS_PROXY="https://proxy.example.com:8080"  # HTTPS over HTTPS proxy

# Windows (Command Prompt)
set HTTPS_PROXY=https://proxy.example.com:8080

# Windows (PowerShell)
$env:HTTPS_PROXY="https://proxy.example.com:8080"
```

The proxy settings will be automatically detected and used for all Deezer Connect connections.

### Hook Scripts

You can use the `--hook` option to specify a script that will be executed when certain events occur. The script receives event information through environment variables.

For all events, the `EVENT` variable contains the event name. Additional variables depend on the specific event:

#### `playing`
Emitted when playback starts

Variables:
- `TRACK_ID`: The ID of the track being played

#### `paused`
Emitted when playback is paused

No additional variables

#### `track_changed`
Emitted when the track changes

Variables:
- `TRACK_ID`: The ID of the track
- `TITLE`: The track title
- `ARTIST`: The main artist name
- `ALBUM_TITLE`: The album title
- `ALBUM_COVER`: The album cover ID, which can be used to construct image URLs:
  ```
  https://e-cdns-images.dzcdn.net/images/cover/{album_cover}/{resolution}x{resolution}.{format}
  ```
  where `{resolution}` is the desired resolution in pixels (up to 1920) and
  `{format}` is either `jpg` (smaller file size) or `png` (higher quality).
  Deezer's default is 500x500.jpg
- `DURATION`: Track duration in seconds

#### `connected`
Emitted when a Deezer client connects to control playback

Variables:
- `USER_ID`: The Deezer user ID
- `USER_NAME`: The Deezer username

#### `disconnected`
Emitted when the controlling Deezer client disconnects

No additional variables

#### Example
All string values are properly escaped for shell safety.

```bash
#!/bin/bash
# example-hook.sh
echo "Event: $EVENT"
case "$EVENT" in
  "track_changed")
    echo "Track changed: $TITLE by $ARTIST"
    ;;
  "connected")
    echo "Connected as: $USER_NAME"
    ;;
esac
```

### Stateless Configuration

**pleezer** operates statelessly and loads user settings, such as normalization and audio quality, when it connects. To apply changes, disconnect and reconnect. This limitation is due to the Deezer Connect protocol.

Command-line options handle settings that cannot be managed statelessly.

### Configuring the Secrets File

For authentication, **pleezer** requires a `secrets.toml` file containing either:

- **email** and **password**: Your Deezer account email address and password, or
- **arl**: The Authentication Reference Link for your Deezer account. If present, this will override the email and password authentication. ARLs expire over time, so using email and password authentication is preferred for long-term access.

In addition to the authentication keys, the `secrets.toml` file can also include the following optional key:

- **bf_secret** (optional): The secret for computing the track decryption key. If not provided, **pleezer** will attempt to extract it from Deezer's public resources. Providing this secret is optional and **pleezer** does not include it to prevent piracy.

**Important:** Keep your `secrets.toml` file secure and private. Do not share it, as it contains sensitive information that can give unauthorized access to your Deezer account.

To obtain the ARL:

1. Visit [Deezer login callback](https://www.deezer.com/desktop/login/electron/callback) and log in.
2. Copy the Authentication Reference Link (ARL) from the button shown. The ARL link will look like `deezer://autolog/...`. You only need the part after `deezer://autolog/` (i.e., `...`).
3. Keep this link confidential as it grants full access to your account.

Here are examples of a `secrets.toml` file:

**Using email and password for authentication:**

```toml
email = "your-email@example.com"
password = "your-password"
```

**Using ARL for authentication (with optional bf_secret):**

```toml
arl = "your-arl"
bf_secret = "your-bf-secret"
```

You can start with the [`secrets.toml.example`](https://github.com/roderickvd/pleezer/blob/main/secrets.toml.example) file provided in the repository as a template.

## Troubleshooting

If you encounter any issues while using **pleezer**, visit our [GitHub Discussions](https://github.com/roderickvd/pleezer/discussions) for help and advice. You can also post your issue there to get help from the community.

## Building pleezer

**pleezer** is supported on Linux and macOS with full compatibility. Windows support is tier two, meaning it is not fully tested and complete compatibility is not guaranteed. Contributions to enhance Windows support are welcome.

### Setting Up Your Build Environment

Before building **pleezer**, make sure your system is set up with a build environment.

#### Linux

1. Install the necessary build tools and dependencies:
  - On Debian/Ubuntu:
    ```bash
    sudo apt-get update
    sudo apt-get install build-essential libasound2-dev pkgconf
    ```
  - On Fedora:
    ```bash
    sudo dnf groupinstall 'Development Tools'
    sudo dnf install alsa-lib-devel
    ```

2. Install Git (optional for the development version):
    ```bash
    sudo apt-get install git  # On Debian/Ubuntu
    sudo dnf install git      # On Fedora
    ```

3. Install the latest version of Rust using [rustup](https://rustup.rs/). Follow the instructions on the rustup website for the most current setup commands.

#### macOS

1. Install the necessary build tools:
  - Install Xcode from the [App Store](https://apps.apple.com/us/app/xcode/id497799835). Then install the Xcode Command Line Tools by running:
    ```bash
    xcode-select --install
    ```

2. Install Rust using [rustup](https://rustup.rs/). Follow the instructions on the rustup website for the most current setup commands.

3. Install Git and Homebrew (optional for the development version):
  - Install Homebrew by following the instructions at [Homebrew's official site](https://brew.sh/).
  - Use Homebrew to install Git:
    ```bash
    brew install git
    ```

#### Windows

1. Install the latest version of Rust using [rustup](https://rustup.rs/). Follow the instructions on the rustup website for the most current setup commands.

2. Set up a build environment by installing Visual Studio with the required components, following the instructions on the [Visual Studio official site](https://visualstudio.microsoft.com/).

3. Install Git (optional for the development version):
  - Download and install Git from the [official site](https://git-scm.com/).

## Contributing

We appreciate and encourage contributions to **pleezer**! Whether you're fixing bugs, adding features, or improving documentation, your involvement is valuable.

### How to Contribute

1. **Submit Issues**: Submit issues for bugs or feature requests by [reporting an issue](https://github.com/roderickvd/pleezer/issues). Include detailed debug logs and be responsive to follow-up questions and testing. Inactive issues may be closed. This is not a general help forum; issues should focus on **pleezer** itself, not your system configuration.

2. **Create Pull Requests**: To contribute code changes or improvements, submit a [pull request](https://github.com/roderickvd/pleezer/pulls). Follow the coding standards in the [Contributing Guidelines](https://github.com/roderickvd/pleezer/blob/main/CONTRIBUTING.md).

3. **Participate in Discussions**: Engage in [GitHub discussions](https://github.com/roderickvd/pleezer/discussions) to offer feedback, stay informed, and collaborate with the community.

For more details on contributing, refer to the [Contributing Guidelines](https://github.com/roderickvd/pleezer/blob/main/CONTRIBUTING.md).

## Support My Work

If you appreciate the effort and dedication put into **pleezer** and other open-source projects, consider supporting my work through GitHub Sponsorships. Your contributions help me continue developing and improving software, and they make a meaningful difference in the sustainability of these projects.

Become a sponsor today at [github.com/sponsors/roderickvd](https://github.com/sponsors/roderickvd). Thank you for your support!

## Related Projects

There are several projects that have influenced **pleezer**. Here are a few:

- [librespot](https://github.com/librespot-org/librespot): An open-source client library for Spotify with support for Spotify Connect.
- [lms-deezer](https://github.com/philippe44/lms-deezer): A plugin for Logitech Media Server to stream music from Deezer.

## License

**pleezer** is licensed under the [Sustainable Use License](https://github.com/roderickvd/pleezer/blob/main/LICENSE.md). This license promotes [fair use](https://faircode.io) and sustainable development of open-source software while preventing unregulated commercial exploitation.

### Non-Commercial Use

You may use, modify, and distribute **pleezer** freely for non-commercial purposes. This includes integrating it into other software or hardware as long as these offerings are available at no cost to users.

### Commercial Use

If you intend to use **pleezer** in a commercial context—such as incorporating it into paid software or hardware products, or any offering that requires payment—you must obtain a separate commercial license. Commercial use includes:

- Bundling **pleezer** with software or hardware that requires payment to unlock features or access.
- Distributing **pleezer** as part of a paid product or service.

For example, **pleezer** can be included in free software or hardware, but if it is part of a product or service that charges for access or additional features, a commercial license is required.

This approach addresses challenges seen with projects like [librespot](https://github.com/librespot-org/librespot), which, despite widespread use, has seen limited contributions. By requiring commercial users to obtain a separate license, we aim to promote fair contributions and support the ongoing development of **pleezer**.

## Important Information

### Important Disclaimer

**pleezer** is an independent project and is not affiliated with, endorsed by, or created by Deezer. It is developed to provide a streaming player that is fully compatible with the Deezer Connect protocol.

**pleezer** **does not and will not support** saving or extracting music files for offline use. This project is committed to respecting artists' rights and strongly opposes piracy. Users must not use **pleezer** to infringe on intellectual property rights.

### Deezer Terms of Service

When using **pleezer**, you must comply with [Deezer's Terms of Service](https://www.deezer.com/legal/cgu). This includes, but is not limited to:

- Using the software only for permitted purposes, such as personal or family use.
- Avoiding any activities that violate Deezer's policies or terms.

It is your responsibility to thoroughly understand and adhere to Deezer's Terms of Service while using **pleezer**.

### Security

For information on how security is handled, including how to report vulnerabilities, please refer to the [Security Policy](https://github.com/roderickvd/pleezer/blob/main/SECURITY.md).

## Contacting the Author

For general inquiries, please use [GitHub Issues](https://github.com/roderickvd/pleezer/issues) or [GitHub Discussions](https://github.com/roderickvd/pleezer/discussions). For commercial licensing or to report security vulnerabilities, you may contact me directly via email. Please avoid using email for general support or feature requests.
