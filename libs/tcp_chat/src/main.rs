use std::env;
use std::io::{self, BufRead, Write};
use std::net::IpAddr;
use std::thread;

use tcp_chat::{ChatConfig, ChatEvent, ChatRuntimeInfo, NetworkInterface, Result};

fn main() -> Result<()> {
    let cli = parse_cli_options()?;
    let display_name = match cli.display_name {
        Some(name) => name,
        None => prompt("Enter your display name: ")?,
    };

    let room_name = cli
        .room
        .or_else(|| env::var("TCP_CHAT_ROOM").ok())
        .unwrap_or_else(|| "default".to_string());
    let shared_secret = cli.secret.or_else(|| env::var("TCP_CHAT_SECRET").ok());
    let bind_addr = cli
        .bind_addr
        .or_else(|| env::var("TCP_CHAT_BIND_ADDR").ok())
        .map(|value| parse_ip_addr("--bind", &value))
        .transpose()?;
    let advertise_addr = cli
        .advertise_addr
        .or_else(|| env::var("TCP_CHAT_ADVERTISE_ADDR").ok())
        .map(|value| parse_ip_addr("--advertise", &value))
        .transpose()?;
    let port = cli
        .port
        .or_else(|| env::var("TCP_CHAT_PORT").ok())
        .map(|value| parse_port("--port", &value))
        .transpose()?;
    let interface_name = cli
        .interface_name
        .or_else(|| env::var("TCP_CHAT_INTERFACE").ok());

    let mut config = ChatConfig::new(display_name)?.with_room(room_name)?;
    if let Some(shared_secret) = shared_secret {
        config = config.with_shared_secret(shared_secret)?;
    }
    if let Some(bind_addr) = bind_addr {
        config = config.with_bind_addr(bind_addr);
    }
    if let Some(advertise_addr) = advertise_addr {
        config = config.with_advertise_addr(advertise_addr);
    }
    if let Some(port) = port {
        config = config.with_port(port);
    }
    if let Some(interface_name) = interface_name {
        config = config.with_interface(interface_name)?;
    }

    let (mut chat, event_rx) = tcp_chat::LanChat::start(config)?;
    let runtime = chat.runtime_info()?;

    println!("tcp_chat is running.");
    print_runtime_summary(&runtime);
    println!("Type a message and press Enter to broadcast it to connected peers.");
    println!("Use /peers, /status, /interfaces, or /quit.");

    let event_thread = thread::spawn(move || {
        while let Ok(event) = event_rx.recv() {
            match event {
                ChatEvent::PeerDiscovered(peer) => {
                    println!("[discovered] {} ({})", peer.display_name, peer.peer_id);
                }
                ChatEvent::PeerConnected(peer) => {
                    println!("[connected] {}", peer.display_name);
                }
                ChatEvent::PeerDisconnected(peer) => {
                    println!("[disconnected] {}", peer.display_name);
                }
                ChatEvent::MessageReceived(message) => {
                    println!("{}: {}", message.display_name, message.body);
                }
                ChatEvent::Warning(message) => {
                    eprintln!("[warning] {message}");
                }
            }
        }
    });

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        print!("> ");
        io::stdout().flush()?;

        let Some(line) = lines.next() else {
            break;
        };
        let line = line?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        match trimmed {
            "/quit" | "/exit" => break,
            "/peers" => {
                let peers = chat.peers();
                if peers.is_empty() {
                    println!("No peers discovered yet.");
                } else {
                    for peer in peers {
                        println!("- {} [{}]", peer.display_name, peer.peer_id);
                    }
                }
            }
            "/status" => {
                print_runtime_summary(&chat.runtime_info()?);
            }
            "/interfaces" => {
                print_interfaces(&tcp_chat::LanChat::local_interfaces()?);
            }
            "/help" => print_help(),
            _ => chat.send(trimmed.to_string())?,
        }
    }

    chat.shutdown();
    drop(chat);
    let _ = event_thread.join();

    Ok(())
}

#[derive(Default)]
struct CliOptions {
    display_name: Option<String>,
    room: Option<String>,
    secret: Option<String>,
    bind_addr: Option<String>,
    advertise_addr: Option<String>,
    port: Option<String>,
    interface_name: Option<String>,
}

fn parse_cli_options() -> Result<CliOptions> {
    let mut cli = CliOptions::default();
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--room" | "-r" => {
                cli.room = Some(next_option_value("--room", &mut args)?);
            }
            "--secret" | "-s" => {
                cli.secret = Some(next_option_value("--secret", &mut args)?);
            }
            "--bind" => {
                cli.bind_addr = Some(next_option_value("--bind", &mut args)?);
            }
            "--advertise" => {
                cli.advertise_addr = Some(next_option_value("--advertise", &mut args)?);
            }
            "--port" | "-p" => {
                cli.port = Some(next_option_value("--port", &mut args)?);
            }
            "--interface" | "-i" => {
                cli.interface_name = Some(next_option_value("--interface", &mut args)?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ if arg.starts_with("--room=") => {
                cli.room = Some(arg["--room=".len()..].to_string());
            }
            _ if arg.starts_with("--secret=") => {
                cli.secret = Some(arg["--secret=".len()..].to_string());
            }
            _ if arg.starts_with("--bind=") => {
                cli.bind_addr = Some(arg["--bind=".len()..].to_string());
            }
            _ if arg.starts_with("--advertise=") => {
                cli.advertise_addr = Some(arg["--advertise=".len()..].to_string());
            }
            _ if arg.starts_with("--port=") => {
                cli.port = Some(arg["--port=".len()..].to_string());
            }
            _ if arg.starts_with("--interface=") => {
                cli.interface_name = Some(arg["--interface=".len()..].to_string());
            }
            _ if cli.display_name.is_none() => {
                cli.display_name = Some(arg);
            }
            _ => {
                return Err(tcp_chat::LanChatError::Config(format!(
                    "unexpected argument: {arg}"
                )));
            }
        }
    }

    Ok(cli)
}

fn next_option_value(option_name: &str, args: &mut impl Iterator<Item = String>) -> Result<String> {
    args.next()
        .ok_or_else(|| tcp_chat::LanChatError::Config(format!("{option_name} requires a value")))
}

fn parse_ip_addr(option_name: &str, value: &str) -> Result<IpAddr> {
    value.parse::<IpAddr>().map_err(|error| {
        tcp_chat::LanChatError::Config(format!("{option_name} must be a valid IP address: {error}"))
    })
}

fn parse_port(option_name: &str, value: &str) -> Result<u16> {
    value.parse::<u16>().map_err(|error| {
        tcp_chat::LanChatError::Config(format!("{option_name} must be a valid TCP port: {error}"))
    })
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;

    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value)
}

fn print_help() {
    println!(
        "Usage: tcp_chat [display_name] [--room ROOM] [--secret SECRET] [--bind IP] [--advertise IP] [--port PORT] [--interface NAME]"
    );
    println!("Environment variables:");
    println!("  TCP_CHAT_ROOM   default room if --room is omitted");
    println!("  TCP_CHAT_SECRET shared secret if --secret is omitted");
    println!("  TCP_CHAT_BIND_ADDR   bind address override");
    println!("  TCP_CHAT_ADVERTISE_ADDR   advertised LAN address override");
    println!("  TCP_CHAT_PORT   TCP port override (defaults to 0 for auto)");
    println!("  TCP_CHAT_INTERFACE   mDNS interface name override (for example en0)");
    println!("Commands:");
    println!("  /peers   list currently known peers");
    println!("  /status  show bind/listen/advertise settings");
    println!("  /interfaces   show active IPv4 interfaces");
    println!("  /quit    exit the client");
    println!("  /help    show this help");
}

fn print_runtime_summary(runtime: &ChatRuntimeInfo) {
    let auth_status = if runtime.auth_required {
        "shared-secret authentication enabled"
    } else {
        "no shared-secret authentication"
    };
    let advertise_mode = runtime
        .advertise_addr
        .map(|address| format!("explicit {address}"))
        .unwrap_or_else(|| "automatic IPv4 interface advertisement".to_string());
    let interface_name = runtime
        .mdns_interface
        .as_deref()
        .unwrap_or("all active IPv4 interfaces");

    println!("Room: {} ({auth_status})", runtime.room_name);
    println!("Peer id: {}", runtime.peer_id);
    println!("Listening on: {}", runtime.listen_addr);
    println!("Bind address: {}", runtime.bind_addr);
    println!("mDNS interface: {interface_name}");
    println!("Advertise mode: {advertise_mode}");
}

fn print_interfaces(interfaces: &[NetworkInterface]) {
    if interfaces.is_empty() {
        println!("No active IPv4 LAN interfaces detected.");
        return;
    }

    println!("Active IPv4 LAN interfaces:");
    for interface in interfaces {
        println!("- {} {}", interface.name, interface.address);
    }
}
