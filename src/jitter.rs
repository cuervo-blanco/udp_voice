use std::collections::VecDeque;
use std::time::{Duration, Instant};
use crate::audio::{FRAME_SIZE, CHANNELS};

// Jitter Buffer Parameters
const JITTER_BUFFER_SIZE: usize = 5;
const PACKET_DURATION_MS: u64 = 20;

pub struct JitterBuffer {
    buffer: VecDeque<Vec<u8>>,
    last_playback_time: Instant,
}

impl JitterBuffer {
    pub fn new() -> Self {
        JitterBuffer {
            buffer: VecDeque::with_capacity(JITTER_BUFFER_SIZE),
            last_playback_time: Instant::now(),
        }
    }

    pub fn add_packet(&mut self, packet: Vec<u8>) {
        if self.buffer.len() < JITTER_BUFFER_SIZE {
            println!("JITTER: Pushing back packet: {:?}", packet);
            self.buffer.push_back(packet);
        } else {
            self.buffer.pop_front();
            self.buffer.push_back(packet);
        }
    }
    pub fn get_next_packet(&mut self) -> Option<Vec<u8>> {
        let now = Instant::now();
        println!("JITTER: Instant: {:?}", now);
        if now.duration_since(self.last_playback_time) >= Duration::from_millis(PACKET_DURATION_MS) {
            self.last_playback_time = now;
            if let Some(packet) = self.buffer.pop_front() {
                println!("JITTER: popped from buffer: {:?}", packet);
                if packet.len() >= FRAME_SIZE * CHANNELS * 2 {
                    println!("JITTER: Packer length: {:?}", packet.len());
                    return Some(packet);
                    
                }
            }
        }
        None
    }
}
