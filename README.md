# UDP Voice

A simple UDP + mDNS application for real-time audio communication over a local network. This project is designed for scenarios requiring fast, reliable, and temporary communication channels—like a film set—where traditional walkie-talkies may be replaced with a smartphone-based solution.

## Overview

This application, developed solely by [Cuervo Blanco](https://github.com/cuervo-blanco), is part of an ongoing project with **Dimitri Médard**, a Film Production Mixer, to create a real-time communication app for iOS and Android. The goal is to establish quick and efficient audio communication using local networking. 

The current codebase runs as a single peer application: every instance can discover peers with mDNS, listen for UDP audio, transmit microphone audio with a command-driven push-to-talk workflow, and exchange LAN text messages through an embedded TCP chat runtime. The networking and buffering are still intentionally simple, but the project now behaves like a usable LAN voice-and-text prototype rather than a split client/server experiment.

## Features

- **Real-time audio streaming** via UDP
- **LAN text messaging** via TCP
- **Peer discovery** over local networks using mDNS
- **Opus audio encoding** for efficient audio compression
- **Interactive Command Line Interface (CLI)**

## Installation

1. **Clone the Repository**
   ```sh
   git clone https://github.com/cuervo-blanco/udp_voice.git
   cd udp_voice
   ```

2. **Install Dependencies**
   Ensure that Rust is installed on your system. This project uses `cargo` for dependency management.
   ```sh
   cargo build
   ```

## Usage

### Running the Peer App

The primary executable is `peer`:

```sh
cargo run --bin peer
```

Legacy `client` and `server` binaries are still present, but they now launch the same peer runtime for compatibility.

Useful startup flags:

```sh
cargo run --bin peer -- --help
cargo run --bin peer -- --list-devices
cargo run --bin peer -- --list-network
cargo run --bin peer -- --setup
cargo run --bin peer -- --username dimitri
cargo run --bin peer -- --input-device "Built-in Microphone" --output-device "MacBook Pro Speakers"
cargo run --bin peer -- --interface en1 --bind-port 18521
```

### Command Interface

Upon launching, the peer opens an interactive terminal setup screen unless you already supplied or saved the core setup values it needs. That screen lets you review the detected system-default mic/speakers, pick named audio devices, choose the LAN interface, set the UDP port, save preferences, refresh the device list, and then start the peer. Audio receive/playback starts immediately after setup, microphone transmission is controlled from the CLI, and text chat starts alongside the audio runtime on the same selected LAN interface.

Available commands:

- `help` - Show the command list
- `peers` - List currently discovered peers
- `select all` - Route microphone audio to every discovered peer
- `select none` - Route microphone audio to nobody
- `select <peer1,peer2>` - Route microphone audio only to named peers
- `talk on` - Start transmitting microphone audio
- `talk off` - Stop transmitting microphone audio
- `talk toggle` - Toggle transmission on or off
- `msg <text>` - Send a text message to currently connected text peers
- `text peers` - List text-chat peers discovered through the embedded TCP chat runtime
- `text status` - Show the text chat room, listener address, and interface
- `stats` - Show packet, jitter, and underflow stats
- `devices` - Show current and available audio devices
- `network` - Show the current network interface, bind address, and UDP port
- `exit` - Exit the application

### Configuration

Audio setup defaults to 48 kHz mono Opus frames and attempts to select usable system input/output devices automatically. For most runs, the startup wizard is the easiest way to choose hardware and networking. If you prefer scripting or already know the device names, you can still use `--list-devices`, `--list-network`, `--input-device`, `--output-device`, `--interface`, and `--bind-port`.

Saved preferences are stored in `.udp_voice_preferences` in the directory where you launch the app. Run with `--setup` any time you want to revisit and update them.

Text messaging currently uses the embedded `libs/tcp_chat` crate with its default room and no shared secret. That keeps the first integration simple and lets every running peer on the same LAN exchange text once it discovers the others.

On macOS, microphone access may also require enabling your terminal app under `System Settings > Privacy & Security > Microphone`.

## Architecture

### mDNS Service

The mDNS module manages peer discovery on the local network. Each peer publishes its UDP listen address and keeps a shared presence table of other visible peers.

### Audio Processing

1. **Audio Capture** - CPAL captures microphone input and downs mixes it to mono frames suitable for Opus.
2. **Encoding and Decoding with Opus** - Opus compresses 20 ms voice frames for low-latency UDP transmission.
3. **Adaptive Jitter Buffering** - The receive path adjusts its packet cushion based on observed arrival jitter and playback underflows.

### Networking

- **Single Peer Runtime** - A single process sends and receives audio while also running a TCP text-chat listener.
- **UDP Socket Communication** - Each peer binds one UDP socket, advertises it over mDNS, and sends Opus packets directly to selected peers.
- **Embedded TCP Chat Runtime** - The in-repo `libs/tcp_chat` crate handles room-scoped LAN text messaging over TCP.
- **mDNS for Device Discovery** - Enables peer presence and peer selection over a local network for both the audio and text layers.

### Debugging

Comprehensive logging is enabled with `log` and `env_logger` crates, providing real-time insights into the application's status, data flow, and errors. These logs assist with debugging packet loss and other network-related challenges.

## Challenges and Future Work

### Challenges
- **Reliability of UDP for Audio** - The inherent packet loss in UDP still needs stronger recovery strategies for harsh Wi-Fi conditions.
- **Cross-Platform Device Handling** - Different host audio stacks expose devices and permissions differently, especially on macOS and mobile.

### Future Work
- **True Push-to-Talk UX** - Replace CLI commands with keyboard, GUI, or mobile touch controls.
- **Duplex Session Controls** - Add mute state, per-peer talk indicators, reconnect handling, and persistent settings.
- **Security Enhancements** - Introduce authentication and encryption for non-trusted networks.

## Acknowledgements

Special thanks to **Dimitri Médard** for his support and expertise in film audio mixing, inspiring this project to provide reliable communication tools for on-set teams.
