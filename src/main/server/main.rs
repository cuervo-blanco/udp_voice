use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::io::Write;
use selflib::mdns_service::MdnsService;
use std::sync::{Arc, Mutex};
use selflib::debug_println;

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


    // -------- Input Thread ------- //
    let (tx, _rx) = channel();
    std::thread::spawn ( move || {
        debug_println!("INPUT: Thread Initialized");
        loop {
            // Take user input
            let reader = std::io::stdin();
            let mut buffer: String = String::new();
            reader.read_line(&mut buffer).unwrap();
            let input = buffer.trim();

            tx.send(input.to_string()).unwrap();
        }
    });

    let ip =  local_ip_address::local_ip().unwrap();
    debug_println!("UDP: Local IP Address: {}", ip);
    let port: u16 = 18522;
    #[allow(unused_variables)]
    let ip_port = format!("{}:{}", ip, port);
    debug_println!("UDP: IP Address & Port: {}", ip);

    // Data Structures
    let _user_table: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(std::collections::HashMap::new()));

    // mDNS
    let properties = vec![("property_1", "attribute_1"), ("property_2", "attribute_2")];
    let mdns = MdnsService::new("_udp_voice._udp.local.", properties);

    mdns.register_service(&instance_name.lock().unwrap(), ip, port);
    mdns.browse_services();
}
