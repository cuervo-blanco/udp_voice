use std::thread;
use std::io::Write;
use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::net::UdpSocket;
use std::sync::{Mutex, Arc};
use audio_sync::audio;
use ringbuf::HeapRb;
use ringbuf::traits::{Split, Producer};
use std::time::Duration;

#[allow(unused_attributes)]
#[macro_use]
macro_rules! debug_println {
    ($($arg:tt)*) => (
        #[cfg(feature = "debug")]
        println!($($arg)*)
    )
}

fn  clear_terminal() {
    print!("\x1B[2J");
    std::io::stdout().flush().unwrap();
}

fn username_take()-> String {
    // Take user input (instance name)
    let reader = std::io::stdin();
    let mut instance_name = String::new();
    reader.read_line(&mut instance_name).unwrap();
    let instance_name = instance_name.replace("\n", "").replace(" ", "_");
    instance_name
}

fn main () {

    println!("");
    println!("Enter Username:");
    // Add validation process? 
    #[allow(unused_variables)]
    let instance_name = Arc::new(Mutex::new(username_take()));
    clear_terminal();

    //---- Audio Setup-----//

    debug_println!("INFO: Instance Name saved as: {:?}", instance_name);
    debug_println!("AUDIO: AUDIO INITIALIZATION IN PROCESS");
    let (Some(input_device), Some(output_device)) = audio::initialize_audio_interface() else {
        debug_println!("AUDIO: AUDIO INITIALIZATION FAILED");
        return;
    };

    let input_config = audio::get_audio_config(&input_device)
        .expect("Failed to get audio input config");
    debug_println!("AUDIO: INPUT AUDIO CONFIG: {:?}", input_config);
    let output_config = audio::get_audio_config(&output_device)
        .expect("Failed to get audio output config");

    // Audio Resources
    debug_println!("AUDIO: OUTPUT AUDIO CONFIG: {:?}", output_config);
    let input_device = Arc::new(Mutex::new(input_device));
    let input_config = Arc::new(Mutex::new(input_config));

    let audio_buffer = HeapRb::<u8>::new(960 * 10);
    let (producer, consumer) = audio_buffer.split();
    let producer = Arc::new(Mutex::new(producer));
    let consumer = Arc::new(Mutex::new(consumer));

    // Data Structures
    let user_table: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(std::collections::HashMap::new()));

    // -------- Input Thread ------- //
    let (tx, rx) = channel();
    std::thread::spawn ( move || {
        debug_println!("INPUT: Thread Initialized");
        loop {
            // Take user input
            let reader = std::io::stdin();
            let mut buffer: String = String::new();
            reader.read_line(&mut buffer).unwrap();
            let input = buffer.trim();

            debug_println!("INPUT: User Input: {}", input);

            if input == "audio.start()" {
                tx.send("audio.start()".to_string()).unwrap();
            } else if input == "audio.stop()"{
                tx.send("audio.stop()".to_string()).unwrap();
            } 
        }
    });


    // Output Stream Init
    debug_println!("AUDIO: Starting output stream");
    #[allow(unused_variables)]
    let output_stream = audio::start_output_stream(
        &output_device,
        &output_config,
        consumer.clone()
    ).expect("AUDIO: Failed to start output stream");
    

    // ----------- UDP Setup and Thread ----------//

    // Create UDP socket
    let ip =  local_ip_address::local_ip().unwrap();
    debug_println!("UDP: Local IP Address: {}", ip);
    let port: u16 = 18522;
    #[allow(unused_variables)]
    let ip_port = format!("{}:{}", ip, port);
    debug_println!("UDP: IP Address & Port: {}", ip);
    let receive_socket = Arc::new(Mutex::new(UdpSocket::bind((ip, port)).expect("UDP: Couldn't bind to address")));
    receive_socket.lock().unwrap().set_nonblocking(true).expect("set non_blocking call failed");

    let send_socket = Arc::new(Mutex::new(UdpSocket::bind("0.0.0.0:0").expect("UDP: Couldn't bind to address")));

    // Create Service

    let producer_clone = Arc::clone(&producer);
    let receive_socket_clone: Arc<Mutex<UdpSocket>> = Arc::clone(&receive_socket);

    thread::spawn( move || {
        debug_println!("UDP: Starting UDP receiver");
        let udp_socket = receive_socket_clone.clone();
        debug_println!("UDP: Receiving in Socket {:?}", udp_socket);
        let mut buffer = [0; 960];
        debug_println!("UDP: Allocated memory for buffering {:?}", buffer);
        loop {
            // debug_println!("UDP: Waiting to acquire socket lock");
            match udp_socket.try_lock() {
                Ok(udp_socket) => { 
                    // debug_println!("UDP: Succesfully acquired socket lock");
                        match udp_socket.recv(&mut buffer) {
                            Ok(size) => {
                                debug_println!("UDP: Amount of bytes received {}", size);
                                match bincode::deserialize::<Vec<u8>>(&buffer[..size]){
                                    Ok(deserialized_data) => match producer_clone.lock() {
                                        Ok(mut producer) => {
                                            debug_println!("Succesfully locked producer");
                                            producer.push_slice(&deserialized_data);
                                        },
                                        Err(e) => {
                                            eprintln!("Failed to lock on to producer {}", e);
                                        }
                                    }, Err(e) => {
                                        eprintln!("Failed to deserialize data: {}", e);
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                // debug_println!("UDP: recv would block");
                            }
                            Err(e) => {
                                eprintln!("Failed to receive data: {}", e);
                            },
                        }
                },
                Err(e) => eprintln!("Failed to lock UDP socket: {}", e)
            }
            // debug_println!("UDP: Looping for next receive");
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    // ---- Sending Audio - Read User Input ----//

    let send_socket_clone: Arc<Mutex<UdpSocket>> = Arc::clone(&send_socket);
    let user_table_clone = Arc::clone(&user_table);

    thread::spawn(move || {
        debug_println!("Starting audio input thread");
        let mut input_stream = None;
        loop {
            match rx.recv() {
                Ok(command) => {
                    if command == "audio.start()" {
                        debug_println!("AUDIO: Starting Input Device");
                        if input_stream.is_none() {
                            match input_device.lock() {
                                Ok(input_device) => match input_config.lock() {
                                    Ok(input_config) => {
                                        match audio::start_input_stream(&input_device, &input_config) {
                                            Ok((stream, receiver)) => {
                                                let receiver = Arc::new(Mutex::new(receiver));
                                                input_stream = Some((stream, Arc::clone(&receiver)));
                                                let udp_socket = Arc::clone(&send_socket_clone);
                                                let user_table = Arc::clone(&user_table_clone);
                                                debug_println!("UDP: Udp Socket stream and receiver initialized");
                                                
                                                thread::spawn(move || {
                                                    debug_println!("UDP: Receiving Thread Initialized");
                                                    loop {
                                                        match receiver.lock() {
                                                            Ok(receiver) => match receiver.recv() {
                                                                Ok(opus_data) => {
                                                                    debug_println!("UDP: Preparing to send opus data");
                                                                    let slice: &[u8] = &opus_data;
                                                                    debug_println!("UDP: Initial Slice to send: {:?}", slice);
                                                                    let encoded_audio: Vec<u8> = bincode::serialize(slice).unwrap();
                                                                    debug_println!("UDP: Encoded Slice to send: {:?}", encoded_audio);

                                                                    let udp_socket_clone = Arc::clone(&udp_socket);
                                                                    let user_table_clone = Arc::clone(&user_table);
                                                                    thread::spawn(move || {
                                                                        for (user, ip) in user_table_clone.lock().unwrap().iter() {
                                                                            let socket_addr = format!("{}:18522", ip);
                                                                            debug_println!("UDP: Connecting to {} on {}", user, socket_addr);
                                                                            let message = format!("Failed to connect to {}", user);
                                                                            if let Err(e) = udp_socket_clone.lock().unwrap().connect(&socket_addr) {
                                                                                eprintln!("{}: {}", message, e);
                                                                            } else {
                                                                                debug_println!("UDP: Sending audio to {}", user);
                                                                                if let Err(e) = udp_socket_clone.lock().unwrap().send(&encoded_audio) {
                                                                                    eprintln!("Failed to send data to {}: {}", user, e);
                                                                                }

                                                                            }
                                                                        }
                                                                    });

                                                                },

                                                                Err(e) => eprintln!("Failed to receive opus data: {}", e),
                                                            },
                                                            Err(e) => eprintln!("Failed to lock receiver: {}", e),

                                                        }
                                                    } 
                                                });
                                            },
                                            Err(e) => eprintln!("AUDIO: Failed to start input stream: {}", e),
                                        }
                                    }
                                    Err(e) => eprintln!("Failed to lock input_config: {}", e),
                                }
                                Err(e) => eprintln!("Failed to lock input device: {}", e),
                            }
                        }
                    } else if command == "audio.stop()" {
                        if let Some((stream, _)) = input_stream.take() {
                            debug_println!("AUDIO STOP COMMAND RECEIVED");
                            audio::stop_audio_stream(stream);
                        }
                    }
                }
                Err(e) => eprintln!("Failed to receive command: {}", e),
            }
        }
    });


    // ----------- mDNS Service Thread ----------//

    // Configure Service
    debug_println!("NET: Commencing mDNS Service");
    let mdns = mdns_sd::ServiceDaemon::new().expect("mDNS: Failed to create daemon");
    let service_type = "_udp_voice._udp.local.";
    debug_println!("NET: Connecting to Local IP address: {}", ip);
    let host_name =  hostname::get()
        .expect("NET: Unable to get host name");
    let host_name = host_name.to_str()
        .expect("NET: Unable to convert to string");
    let host_name = format!("{}.local.", host_name);
    debug_println!("NET: Host name: {}", host_name);
    let properties = [("property_1", "attribute_1"), ("property_2", "attribute_2")];
    let instance_name_clone = instance_name.clone();
    let instance_name_clone = instance_name_clone.lock().unwrap();

    let udp_voice_service = mdns_sd::ServiceInfo::new(
        service_type,
        &instance_name_clone,
        host_name.as_str(),
        ip,
        port,
        &properties[..],
    ).unwrap();
    debug_println!("mDNS: Service Info created: {:?}", udp_voice_service);
    // Broadcast service
    mdns.register(udp_voice_service).expect("Failed to register service");
    debug_println!("mDNS: Service registered");

    // Query for Services
    let receiver = mdns.browse(service_type).expect("Failed to browse");
    debug_println!("mDNS: Browsing for services: {:?}", receiver);

    debug_println!("mDNS: Starting mDNS service thread...");
    // Listen for Services, Respond & Store
    loop {
        debug_println!("mDNS: Starting mDNS loop");
            while let Ok(event) = receiver.recv() {
                match event {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        debug_println!("mDNS: Service resolved: {:?}", info);
                        // Send request to create udp connection
                        let addresses = info.get_addresses_v4();
                        debug_println!("mDNS: Addresses found: {:?}", addresses);
                        for address in addresses {
                            debug_println!("mDNS: Found User in IP Address: {:?}", address);
                            // --------- Udp Connection ---------//
                            user_table.lock().unwrap().insert(info.get_fullname().to_string(), address.to_string());
                            debug_println!("mDNS: Inserted New User Into User Table: {:?}", info.get_fullname());
                            let mut username = String::new();
                            debug_println!("mDNS: Username: {:?}", username);
                            for char in info.get_fullname().chars() {
                                if char != '.' {
                                    username.push(char);
                                } else {
                                    break;
                                }
                            }
                            debug_println!("{} just connected", username);
                        }
                    },
                    _ => {

                    }
                }
            }
            debug_println!("mDNS: Restarting Loop");
    }
}
