#[allow(unused_imports)]
use std::io::{stdout, Write};

pub fn clear_terminal() {
    print!("\x1B[2J\x1B[H");
    std::io::stdout().flush().unwrap();
}

pub fn sanitize_username(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                '_'
            } else {
                character
            }
        })
        .collect()
}

pub fn username_take() -> String {
    // Take user input (instance name)
    let reader = std::io::stdin();
    let mut instance_name = String::new();
    reader.read_line(&mut instance_name).unwrap();
    sanitize_username(&instance_name)
}
