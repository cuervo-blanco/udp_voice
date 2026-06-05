# tcp_chat

`tcp_chat` is now structured as a reusable Rust library plus a small reference CLI for LAN messaging over TCP with mDNS discovery.

## What It Does
- Discovers peers on the local network with mDNS.
- Filters discovery by room so unrelated peers on the same LAN do not connect.
- Supports optional shared-secret authentication with a mutual hello handshake.
- Exchanges newline-delimited JSON messages over TCP.
- Defaults to IPv4-oriented LAN discovery and can pin discovery to a named interface.
- Exposes a library API that other terminal or GUI apps can embed.
- Ships with a simple CLI so you can test and use it immediately.

## Project Layout
- `src/lib.rs`: reusable LAN chat runtime and public API.
- `src/auth.rs`: room identity and shared-secret proof generation.
- `src/protocol.rs`: portable wire protocol and frame validation.
- `src/error.rs`: shared error types.
- `src/main.rs`: thin terminal client built on top of the library.

## Why This Is Better Than The Original Prototype
- The wire protocol is explicit and portable instead of relying on ad hoc string parsing.
- Message framing is safe for TCP streams.
- The socket listener binds to `0.0.0.0` by default instead of one guessed local address.
- Discovery is room-scoped, so peers only see compatible rooms.
- The connection is not treated as live until both sides complete an authenticated hello exchange.
- Peer discovery skips self-connections and uses deterministic connection ownership to reduce duplicates.
- The crate now has tests for protocol behavior, auth proof validation, and peer metadata merging.
- The terminal client is separated from the networking engine, so apps can reuse the library without forking the CLI.

## Running The CLI
```sh
cargo run --offline -- "your name" --room studio-a --secret "shared-room-key"
```

If you omit the name, the CLI prompts for one. `--room` defaults to `default`. `--secret` is optional.

Useful options for real LAN testing:
- `--interface en0`: only use one network interface for mDNS discovery.
- `--advertise 192.168.1.23`: force the advertised IPv4 address.
- `--bind 0.0.0.0`: listen on all IPv4 interfaces.
- `--port 0`: let the OS choose a free TCP port. This is the default.

You can also use environment variables:
- `TCP_CHAT_ROOM`
- `TCP_CHAT_SECRET`
- `TCP_CHAT_INTERFACE`
- `TCP_CHAT_BIND_ADDR`
- `TCP_CHAT_ADVERTISE_ADDR`
- `TCP_CHAT_PORT`

Commands:
- `/peers`: list currently known peers
- `/status`: show the current room, peer id, listen address, bind address, and mDNS mode
- `/interfaces`: list active IPv4 LAN interfaces
- `/help`: show CLI help
- `/quit`: exit the client

## Two-Computer LAN Checklist
For two different computers on the same local network, start simple:

```sh
cargo run --offline -- "studio-a" --room control
cargo run --offline -- "studio-b" --room control
```

If either machine has Wi-Fi, Ethernet, VPN, or virtual adapters at the same time, pin the LAN interface explicitly:

```sh
cargo run --offline -- "studio-a" --room control --interface en0
```

If you already know the machine's LAN IPv4 address, forcing it is even more explicit:

```sh
cargo run --offline -- "studio-a" --room control --advertise 192.168.1.23 --bind 0.0.0.0
```

## Troubleshooting
If two machines still do not discover each other, the common causes are environmental rather than protocol-level:
- the two apps are not using the same room
- one app has `--secret` enabled and the other does not, or the secrets differ
- the OS firewall blocks incoming TCP for the terminal or app that launched `tcp_chat`
- the network blocks or isolates mDNS multicast traffic, which is common on guest Wi-Fi, some VPNs, and some managed routers
- the wrong interface is being used for discovery

The CLI now helps surface that state:
- `/status` shows exactly what address and port the node is listening on
- `/interfaces` shows the available IPv4 LAN interfaces you can target with `--interface`
- startup warnings will mention when multiple active LAN interfaces are present

For macOS specifically, if the binary or terminal app is blocked by the Application Firewall, allow incoming connections for the terminal or packaged app and then retry.

## Using The Library
```rust
use tcp_chat::{ChatConfig, ChatEvent, LanChat};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ChatConfig::new("Studio Mac")?
        .with_room("control-room")?
        .with_shared_secret("shared-room-key")?
        .with_interface("en0")?;
    let (chat, events) = LanChat::start(config)?;

    std::thread::spawn(move || {
        while let Ok(event) = events.recv() {
            if let ChatEvent::MessageReceived(message) = event {
                println!("{}: {}", message.display_name, message.body);
            }
        }
    });

    chat.send("hello from the library")?;
    Ok(())
}
```

## Production Notes
This refactor turns the repo into a much safer foundation, but a polished production app should still add a few things on top:
- a proper terminal UI or GUI event layer
- reconnect/backoff policy
- a bounded async runtime instead of one thread per connection
- integration tests with multiple live peers
- richer message types for control, presence, and file or audio signaling

## Development
Useful checks:

```sh
cargo check --offline
cargo test --offline
cargo clippy --offline --all-targets --all-features
```
