# UDP Voice

A simple UDP + mDNS application for real-time audio communication over a local network. This project is designed for scenarios requiring fast, reliable, and temporary communication channels—like a film set—where traditional walkie-talkies may be replaced with a smartphone-based solution.

## Overview

This application, developed solely by [Cuervo Blanco](https://github.com/cuervo-blanco), is part of an ongoing project with **Dimitri Médard**, a Film Production Mixer, to create a real-time communication app for iOS and Android. The goal is to establish quick and efficient audio communication using local networking. 

Currently, the mDNS (multicast DNS) discovery functionality is operational, allowing users to connect and see other users on the same network. While this real-time discovery of peers over a network is functional, the UDP audio streaming component is still under testing to optimize for reliability and low latency.

**Note:** This project is on hold as we research solutions to enhance UDP reliability and security. UDP’s packet loss can be problematic for real-time audio but remains the fastest protocol for this use case.

## Features

- **Real-time audio streaming** via UDP
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

### Running the Application

The project provides CLI executables for different roles:
- **Client** (`src/main/client/main.rs`): Establishes communication by connecting to other peers on the network.
- **Server** (`src/main/server/main.rs`): Manages audio reception and playback.
- **Sine** (`src/main/sine/main.rs`): Generates a sine wave for audio testing.
- **Test** (`src/main/test/main.rs`): Runs general application tests.

Each of these can be run with:
```sh
cargo run --bin <binary_name>
```

For example:
```sh
cargo run --bin client
```

### Command Interface

Upon launching, the client prompts you to:
1. **Enter a Username** - This username will display to other users on the network.
2. **Enter Commands** - Supported commands:
   - `send` - Starts the audio streaming process.
   - `exit` - Exits the application.

### Configuration

Settings are configured in the code through the `Settings` struct. Key configurations include:
- **Sample rate** and **buffer size** for audio quality.
- **Channels** for mono or stereo configurations.

## Architecture

### mDNS Service

The mDNS module manages peer discovery on the local network. Each client instance registers its presence, allowing other instances to detect new connections. This is crucial for a distributed communication system where multiple devices need to identify and connect with each other.

### Audio Processing

1. **Audio Generation and Capture** - The `sine` module allows testing with a generated sine wave to simulate audio input.
2. **Encoding and Decoding with Opus** - Opus is used to compress audio data before transmission, optimizing bandwidth usage without sacrificing audio quality.
3. **Buffer Management** - Ring buffers ensure smooth audio streaming by maintaining data flow between encoding, decoding, and playback processes.

### Networking

- **UDP Socket Communication** - A UDP socket facilitates low-latency transmission, though UDP does not guarantee delivery or order of packets, which can affect audio quality. 
- **mDNS for Device Discovery** - Enables seamless peer-to-peer connections over a local network.

### Debugging

Comprehensive logging is enabled with `log` and `env_logger` crates, providing real-time insights into the application's status, data flow, and errors. These logs assist with debugging packet loss and other network-related challenges.

## Challenges and Future Work

### Challenges
- **Reliability of UDP for Audio** - The inherent packet loss in UDP is a major challenge for real-time audio. Exploring fallback options or adding redundancy mechanisms is under consideration.
- **Debugging Network Issues** - Issues with packet transmission and loss require tools and strategies for efficient debugging.

### Future Work
- **iOS and Android Integration** - Extend the application to work on mobile platforms, allowing devices to function as walkie-talkies.
- **Improved Audio Quality** - Optimize audio encoding settings to improve quality without adding latency.
- **Security Enhancements** - Introduce measures to prevent packet interception or other security vulnerabilities.

## Acknowledgements

Special thanks to **Dimitri Médard** for his support and expertise in film audio mixing, inspiring this project to provide reliable communication tools for on-set teams.
