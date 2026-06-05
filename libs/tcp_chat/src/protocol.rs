use std::io::BufRead;

use serde::{Deserialize, Serialize};

use crate::error::{LanChatError, Result};

pub const MAX_FRAME_BYTES: usize = 8 * 1024;
pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WireEnvelope {
    pub version: u8,
    pub room_id: String,
    pub peer_id: String,
    pub display_name: String,
    #[serde(flatten)]
    pub message: WireMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireMessage {
    Hello { auth_proof: Option<String> },
    Text { body: String },
}

impl WireEnvelope {
    pub fn hello(
        peer_id: impl Into<String>,
        display_name: impl Into<String>,
        room_id: impl Into<String>,
        auth_proof: Option<String>,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            room_id: room_id.into(),
            peer_id: peer_id.into(),
            display_name: display_name.into(),
            message: WireMessage::Hello { auth_proof },
        }
    }

    pub fn text(
        peer_id: impl Into<String>,
        display_name: impl Into<String>,
        room_id: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            room_id: room_id.into(),
            peer_id: peer_id.into(),
            display_name: display_name.into(),
            message: WireMessage::Text { body: body.into() },
        }
    }
}

pub fn encode_frame(envelope: &WireEnvelope) -> Result<Vec<u8>> {
    let mut frame = serde_json::to_vec(envelope)?;
    if frame.len() > MAX_FRAME_BYTES {
        return Err(LanChatError::Protocol(format!(
            "frame exceeds maximum size of {MAX_FRAME_BYTES} bytes"
        )));
    }

    frame.push(b'\n');
    Ok(frame)
}

pub fn decode_frame(line: &str) -> Result<WireEnvelope> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(LanChatError::Protocol(format!(
            "frame exceeds maximum size of {MAX_FRAME_BYTES} bytes"
        )));
    }

    let frame = line.trim_end_matches('\n');
    let envelope: WireEnvelope = serde_json::from_str(frame)?;
    if envelope.version != PROTOCOL_VERSION {
        return Err(LanChatError::Protocol(format!(
            "unsupported protocol version {}",
            envelope.version
        )));
    }

    Ok(envelope)
}

pub fn read_frame<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<Option<WireEnvelope>> {
    buffer.clear();

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if buffer.is_empty() {
                Ok(None)
            } else {
                Err(LanChatError::Protocol(
                    "connection closed before a full frame was received".to_string(),
                ))
            };
        }

        let chunk_len = match available.iter().position(|byte| *byte == b'\n') {
            Some(newline_index) => newline_index + 1,
            None => available.len(),
        };

        if buffer.len() + chunk_len > MAX_FRAME_BYTES + 1 {
            reader.consume(chunk_len);
            return Err(LanChatError::Protocol(format!(
                "frame exceeds maximum size of {MAX_FRAME_BYTES} bytes"
            )));
        }

        buffer.extend_from_slice(&available[..chunk_len]);
        reader.consume(chunk_len);

        if buffer.last() == Some(&b'\n') {
            let line = std::str::from_utf8(buffer).map_err(|error| {
                LanChatError::Protocol(format!("frame is not valid UTF-8: {error}"))
            })?;

            return decode_frame(line).map(Some);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        decode_frame, encode_frame, read_frame, WireEnvelope, WireMessage, MAX_FRAME_BYTES,
    };

    #[test]
    fn text_packets_round_trip() {
        let message = WireEnvelope::text("peer-1", "Cuervo", "room-1", "hello LAN");
        let encoded = encode_frame(&message).expect("encode should succeed");
        let decoded = decode_frame(std::str::from_utf8(&encoded).expect("utf8 frame"))
            .expect("decode should succeed");

        assert_eq!(decoded, message);
    }

    #[test]
    fn oversized_frames_are_rejected() {
        let message = WireEnvelope::text("peer-1", "Cuervo", "room-1", "x".repeat(MAX_FRAME_BYTES));
        assert!(encode_frame(&message).is_err());
    }

    #[test]
    fn hello_packets_are_supported() {
        let message =
            WireEnvelope::hello("peer-2", "Lan Peer", "room-2", Some("proof".to_string()));
        let encoded = encode_frame(&message).expect("encode should succeed");
        let decoded = decode_frame(std::str::from_utf8(&encoded).expect("utf8 frame"))
            .expect("decode should succeed");

        assert!(matches!(
            decoded.message,
            WireMessage::Hello {
                auth_proof: Some(_)
            }
        ));
    }

    #[test]
    fn bounded_reader_decodes_a_single_frame() {
        let message = WireEnvelope::text("peer-1", "Cuervo", "room-1", "frame");
        let encoded = encode_frame(&message).expect("encode should succeed");
        let mut cursor = Cursor::new(encoded);
        let mut scratch = Vec::new();

        let decoded = read_frame(&mut cursor, &mut scratch)
            .expect("read should succeed")
            .expect("frame should exist");

        assert_eq!(decoded, message);
    }

    #[test]
    fn bounded_reader_rejects_oversized_frames() {
        let oversized = format!("{}\n", "x".repeat(MAX_FRAME_BYTES + 1));
        let mut cursor = Cursor::new(oversized.into_bytes());
        let mut scratch = Vec::new();

        assert!(read_frame(&mut cursor, &mut scratch).is_err());
    }
}
