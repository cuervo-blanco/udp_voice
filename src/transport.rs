use crate::settings::MAX_OPUS_PACKET_SIZE;
use std::{
    error::Error,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

pub const PACKET_HEADER_SIZE: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPacket {
    pub sequence_number: u32,
    pub timestamp_ms: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketError {
    PayloadTooLarge(usize),
    PacketTooSmall(usize),
    LengthMismatch { expected: usize, actual: usize },
}

impl fmt::Display for PacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge(size) => write!(
                formatter,
                "payload of {size} bytes exceeds the maximum Opus packet size of {MAX_OPUS_PACKET_SIZE} bytes"
            ),
            Self::PacketTooSmall(size) => {
                write!(formatter, "packet is too small to contain the audio header: {size} bytes")
            }
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "packet length mismatch: expected {expected} bytes but received {actual} bytes"
            ),
        }
    }
}

impl Error for PacketError {}

pub fn serialize_packet(
    sequence_number: u32,
    timestamp_ms: u64,
    payload: &[u8],
) -> Result<Vec<u8>, PacketError> {
    if payload.len() > MAX_OPUS_PACKET_SIZE {
        return Err(PacketError::PayloadTooLarge(payload.len()));
    }

    let mut packet = Vec::with_capacity(PACKET_HEADER_SIZE + payload.len());
    packet.extend_from_slice(&sequence_number.to_be_bytes());
    packet.extend_from_slice(&timestamp_ms.to_be_bytes());
    packet.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    packet.extend_from_slice(payload);

    Ok(packet)
}

pub fn deserialize_packet(bytes: &[u8]) -> Result<AudioPacket, PacketError> {
    if bytes.len() < PACKET_HEADER_SIZE {
        return Err(PacketError::PacketTooSmall(bytes.len()));
    }

    let sequence_number = u32::from_be_bytes(bytes[0..4].try_into().expect("slice length checked"));
    let timestamp_ms = u64::from_be_bytes(bytes[4..12].try_into().expect("slice length checked"));
    let payload_len =
        u16::from_be_bytes(bytes[12..14].try_into().expect("slice length checked")) as usize;
    let expected_len = PACKET_HEADER_SIZE + payload_len;

    if bytes.len() != expected_len {
        return Err(PacketError::LengthMismatch {
            expected: expected_len,
            actual: bytes.len(),
        });
    }

    Ok(AudioPacket {
        sequence_number,
        timestamp_ms,
        payload: bytes[PACKET_HEADER_SIZE..].to_vec(),
    })
}

pub fn current_time_in_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{deserialize_packet, serialize_packet, PacketError};

    #[test]
    fn packet_round_trip_preserves_fields() {
        let payload = vec![1, 2, 3, 4, 5];
        let packet = serialize_packet(42, 123_456, &payload).expect("packet should serialize");
        let decoded = deserialize_packet(&packet).expect("packet should deserialize");

        assert_eq!(decoded.sequence_number, 42);
        assert_eq!(decoded.timestamp_ms, 123_456);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn truncated_packet_is_rejected() {
        let error = deserialize_packet(&[0; 8]).expect_err("packet should be rejected");
        assert_eq!(error, PacketError::PacketTooSmall(8));
    }

    #[test]
    fn mismatched_payload_length_is_rejected() {
        let mut packet = serialize_packet(7, 99, &[1, 2, 3]).expect("packet should serialize");
        packet.pop();

        let error = deserialize_packet(&packet).expect_err("packet should be rejected");
        assert_eq!(
            error,
            PacketError::LengthMismatch {
                expected: 17,
                actual: 16,
            }
        );
    }
}
