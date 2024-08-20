#[allow(unused_imports)]
use std::io::{Write, stdout};

pub fn  clear_terminal() {
    print!("\x1B[2J");
    std::io::stdout().flush().unwrap();
}

pub fn username_take()-> String {
    // Take user input (instance name)
    let reader = std::io::stdin();
    let mut instance_name = String::new();
    reader.read_line(&mut instance_name).unwrap();
    let instance_name = instance_name.replace("\n", "").replace(" ", "_");
    instance_name
}

