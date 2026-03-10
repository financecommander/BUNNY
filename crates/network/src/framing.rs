use crate::error::{NetworkError, Result};

/// Frame types for the wire protocol.
///
/// ```text
/// [frame_type: 1B] [length: 4B BE] [payload: length bytes]
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Handshake initiation (contains serialized HandshakeInit).
    HandshakeInit = 0x01,
    /// Handshake acceptance (contains serialized HandshakeAccept).
    HandshakeAccept = 0x02,
    /// Standard encrypted envelope (SwarmEnvelope bytes).
    Envelope = 0x03,
    /// Cloaked encrypted envelope (CloakedEnvelope bytes).
    CloakedEnvelope = 0x04,
}

impl FrameType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0x01 => Ok(Self::HandshakeInit),
            0x02 => Ok(Self::HandshakeAccept),
            0x03 => Ok(Self::Envelope),
            0x04 => Ok(Self::CloakedEnvelope),
            _ => Err(NetworkError::InvalidFrameType(v)),
        }
    }
}

/// A framed message for the wire protocol.
#[derive(Debug, Clone)]
pub struct Frame {
    pub frame_type: FrameType,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(frame_type: FrameType, payload: Vec<u8>) -> Self {
        Self {
            frame_type,
            payload,
        }
    }

    /// Serialize frame to wire format: [type: 1B] [length: 4B BE] [payload].
    pub fn to_bytes(&self) -> Vec<u8> {
        let len = self.payload.len() as u32;
        let mut buf = Vec::with_capacity(5 + self.payload.len());
        buf.push(self.frame_type as u8);
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Parse a frame from wire bytes.
    ///
    /// Returns the parsed frame and the number of bytes consumed.
    pub fn from_bytes(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < 5 {
            return Err(NetworkError::FramingError(
                "frame too short for header".into(),
            ));
        }

        let frame_type = FrameType::from_u8(data[0])?;
        let len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

        if data.len() < 5 + len {
            return Err(NetworkError::FramingError(format!(
                "frame payload truncated: expected {len} bytes, have {}",
                data.len() - 5
            )));
        }

        let payload = data[5..5 + len].to_vec();
        Ok((Self { frame_type, payload }, 5 + len))
    }
}

/// Encode a frame and write it to a QUIC send stream.
pub async fn write_frame(
    send: &mut quinn::SendStream,
    frame: &Frame,
) -> Result<()> {
    let bytes = frame.to_bytes();
    send.write_all(&bytes).await.map_err(NetworkError::QuinnWrite)?;
    Ok(())
}

/// Read a complete frame from a QUIC receive stream.
pub async fn read_frame(
    recv: &mut quinn::RecvStream,
    max_frame_size: u32,
) -> Result<Frame> {
    // Read header: 1 byte type + 4 bytes length
    let mut header = [0u8; 5];
    recv.read_exact(&mut header)
        .await
        .map_err(|e| NetworkError::FramingError(format!("failed to read frame header: {e}")))?;

    let frame_type = FrameType::from_u8(header[0])?;
    let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);

    if len > max_frame_size {
        return Err(NetworkError::FramingError(format!(
            "frame size {len} exceeds maximum {max_frame_size}"
        )));
    }

    // Read payload
    let mut payload = vec![0u8; len as usize];
    recv.read_exact(&mut payload)
        .await
        .map_err(|e| NetworkError::FramingError(format!("failed to read frame payload: {e}")))?;

    Ok(Frame {
        frame_type,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let frame = Frame::new(FrameType::Envelope, b"hello world".to_vec());
        let bytes = frame.to_bytes();
        let (parsed, consumed) = Frame::from_bytes(&bytes).unwrap();

        assert_eq!(consumed, bytes.len());
        assert_eq!(parsed.frame_type, FrameType::Envelope);
        assert_eq!(parsed.payload, b"hello world");
    }

    #[test]
    fn frame_types() {
        assert_eq!(FrameType::from_u8(0x01).unwrap(), FrameType::HandshakeInit);
        assert_eq!(FrameType::from_u8(0x02).unwrap(), FrameType::HandshakeAccept);
        assert_eq!(FrameType::from_u8(0x03).unwrap(), FrameType::Envelope);
        assert_eq!(FrameType::from_u8(0x04).unwrap(), FrameType::CloakedEnvelope);
        assert!(FrameType::from_u8(0xFF).is_err());
    }

    #[test]
    fn frame_header_layout() {
        let frame = Frame::new(FrameType::HandshakeInit, vec![0xAA; 256]);
        let bytes = frame.to_bytes();

        assert_eq!(bytes[0], 0x01); // HandshakeInit
        assert_eq!(&bytes[1..5], &[0, 0, 1, 0]); // length 256 in BE
        assert_eq!(bytes.len(), 5 + 256);
    }

    #[test]
    fn truncated_frame_rejected() {
        let frame = Frame::new(FrameType::Envelope, vec![0; 100]);
        let bytes = frame.to_bytes();
        // Truncate
        assert!(Frame::from_bytes(&bytes[..50]).is_err());
    }

    #[test]
    fn empty_payload() {
        let frame = Frame::new(FrameType::Envelope, vec![]);
        let bytes = frame.to_bytes();
        let (parsed, consumed) = Frame::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, 5);
        assert!(parsed.payload.is_empty());
    }
}
