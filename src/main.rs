use std::thread;
use std::io::Write;
use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::net::UdpSocket;
use std::sync::{Mutex, Arc};
use audio_sync::audio;

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
    let audio_buffer = Arc::new(Mutex::new(Vec::new()));
    debug_println!("AUDIO: Allocated audio buffer: {:?}", audio_buffer);
    let received_data = Arc::clone(&audio_buffer);

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


    

    // ----------- UDP Setup and Thread ----------//

    // Create UDP socket
    let ip =  local_ip_address::local_ip().unwrap();
    let port: u16 = 18522;
    let ip_port = format!("{}:{}", ip, port);
    let udp_socket = Arc::new(Mutex::new(UdpSocket::bind(ip_port).expect("UDP: Couldn't bind to address")));
    // Create Service

    let received_data_clone = Arc::clone(&received_data);
    let udp_socket_clone: Arc<Mutex<UdpSocket>> = Arc::clone(&udp_socket);

    thread::spawn( move || {
        debug_println!("UDP: Starting UDP reception");
        let udp_socket = udp_socket_clone.clone();
        let mut buffer = [0; 960];
        loop {
            match udp_socket.lock().unwrap().recv(&mut buffer) {
                Ok(size) => {
                    debug_println!("UDP: Amount of bytes received {}", size);
                    let mut data = received_data_clone.lock().unwrap();
                    data.extend_from_slice(&buffer[..size]);
                }
                Err(e) => {
                    eprintln!("Failed to receive data: {}", e);
                }
            }
        }
    });

    for (user, socket) in user_table.lock().unwrap().iter() {
        debug_println!("UDP: Connecting to {} on {}", user, socket);
        let message = format!("Failed to connect to {}", user);
        udp_socket.lock().unwrap().connect(socket).expect(&message);
    };
    // ---- Sending Audio - Read User Input ----//

    let udp_socket_clone_2: Arc<Mutex<UdpSocket>> = Arc::clone(&udp_socket);
    let user_table_clone = Arc::clone(&user_table);
    let instance_name_copy = Arc::clone(&instance_name);

    thread::spawn(move || {
        debug_println!("Starting audio input thread");
        let mut input_stream = None;
        loop {
            let command = rx.recv().unwrap();
            if command == "audio.start()" {
                debug_println!("AUDIO: Starting Input Device");
                if input_stream.is_none() {
                    let (stream, receiver) = 
                        audio::start_input_stream(
                            &input_device.lock().unwrap(),
                            &input_config.lock().unwrap()
                        )
                        .expect("UDP: Failed to start input stream");
                    let receiver = Arc::new(Mutex::new(receiver));
                    input_stream = Some((stream, Arc::clone(&receiver)));
                    let udp_socket_2 = Arc::clone(&udp_socket_clone_2);
                    let user_table = Arc::clone(&user_table_clone);
                    let instance_name = Arc::clone(&instance_name_copy);
                    debug_println!("UDP: Udp Socket stream and receiver initialized");
                    thread::spawn(move || {
                        loop {
                            // Handle encoded data (await)
                            if let Ok(opus_data) = receiver.lock().unwrap().recv() {
                                let slice: &[u8] = &opus_data;
                                debug_println!("UDP: Opus Data received: {:?}", slice);
                                let udp_socket_2 = udp_socket_2.lock().unwrap();
                                let user_table_2 = user_table.lock().unwrap();
                                for (user, socket) in user_table_2.iter() {
                                    debug_println!("UDP: Sending audio to {}", user);
                                    if *user == *instance_name.lock().unwrap() {
                                        continue;
                                    }
                                    udp_socket_2.send_to(slice, socket).expect("Failed to send data");
                                };
                            }
                        } 
                    });
                }
            } else if command == "audio.stop()" {
                if let Some((stream, _)) = input_stream.take() {
                    debug_println!("AUDIO STOP COMMAND RECEIVED");
                    audio::stop_audio_stream(stream);
                }
            }
        }
    });

    debug_println!("AUDIO: Starting output stream");
    #[allow(unused_variables)]
    let output_stream = audio::start_output_stream(
        &output_device,
        &output_config,
        received_data.clone()
    ).expect("MAIN: Failed to start output stream");


    // ----------- mDNS Service Thread ----------//

    // Configure Service
    debug_println!("NET: Commencing mDNS Service");
    let mdns = mdns_sd::ServiceDaemon::new().expect("mDNS: Failed to create daemon");
    let service_type = "_udp_voice._tcp.local.";
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
                        // Send request to create tcp connection
                        let addresses = info.get_addresses_v4();
                        debug_println!("mDNS: Addresses found: {:?}", addresses);
                        for address in addresses {
                            let user_socket = format!("{}:{}", address, info.get_port());
                            debug_println!("mDNS: UDP Socket: {:?}", user_socket);
                            // --------- Udp Connection ---------//
                            user_table.lock().unwrap().insert(info.get_fullname().to_string(), user_socket);
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
            debug_println!("THREAD 3: Restarting Loop");
    }
}
