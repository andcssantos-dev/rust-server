use aurenfall_contracts::{BootstrapCodecError, FRAME_HEADER_BYTES, FrameHeader, MessageKind};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DatagramFrameError {
    #[error(transparent)]
    Codec(#[from] BootstrapCodecError),
    #[error("datagram frame {actual} bytes exceeds configured maximum {maximum}")]
    TooLarge { actual: usize, maximum: usize },
    #[error("datagram frame is shorter than the {minimum}-byte header: {actual} bytes")]
    HeaderTooShort { actual: usize, minimum: usize },
    #[error("datagram payload length mismatch: header declares {declared} bytes, frame contains {actual}")]
    PayloadLengthMismatch { declared: usize, actual: usize },
}

pub fn encode_datagram_frame(
    kind: MessageKind,
    payload: &[u8],
    maximum_datagram_bytes: usize,
) -> Result<Vec<u8>, DatagramFrameError> {
    let header = FrameHeader::new(kind, payload.len())?.encode();
    let total_len = FRAME_HEADER_BYTES + payload.len();
    if total_len > maximum_datagram_bytes {
        return Err(DatagramFrameError::TooLarge {
            actual: total_len,
            maximum: maximum_datagram_bytes,
        });
    }

    let mut frame = Vec::with_capacity(total_len);
    frame.extend_from_slice(&header);
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn decode_datagram_frame(
    frame: &[u8],
    maximum_datagram_bytes: usize,
) -> Result<(MessageKind, &[u8]), DatagramFrameError> {
    if frame.len() > maximum_datagram_bytes {
        return Err(DatagramFrameError::TooLarge {
            actual: frame.len(),
            maximum: maximum_datagram_bytes,
        });
    }
    if frame.len() < FRAME_HEADER_BYTES {
        return Err(DatagramFrameError::HeaderTooShort {
            actual: frame.len(),
            minimum: FRAME_HEADER_BYTES,
        });
    }

    let header_bytes: [u8; FRAME_HEADER_BYTES] =
        frame[..FRAME_HEADER_BYTES]
            .try_into()
            .map_err(|_| DatagramFrameError::HeaderTooShort {
                actual: frame.len(),
                minimum: FRAME_HEADER_BYTES,
            })?;
    let header = FrameHeader::decode(header_bytes)?;
    let payload = &frame[FRAME_HEADER_BYTES..];
    let declared = usize::from(header.payload_len);
    if declared != payload.len() {
        return Err(DatagramFrameError::PayloadLengthMismatch {
            declared,
            actual: payload.len(),
        });
    }

    Ok((header.kind, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_frame_roundtrips() -> Result<(), DatagramFrameError> {
        let payload = [1_u8, 2, 3, 4];
        let encoded = encode_datagram_frame(MessageKind::MoveIntent, &payload, 1200)?;
        let (kind, decoded) = decode_datagram_frame(&encoded, 1200)?;

        assert_eq!(kind, MessageKind::MoveIntent);
        assert_eq!(decoded, payload);
        Ok(())
    }

    #[test]
    fn oversized_datagram_is_rejected() {
        let payload = vec![0_u8; 1200];
        assert!(matches!(
            encode_datagram_frame(MessageKind::MoveIntent, &payload, 1200),
            Err(DatagramFrameError::TooLarge { .. })
        ));
    }

    #[test]
    fn payload_length_mismatch_is_rejected() -> Result<(), DatagramFrameError> {
        let mut encoded = encode_datagram_frame(MessageKind::MoveIntent, &[1_u8, 2], 1200)?;
        encoded.push(3);
        assert!(matches!(
            decode_datagram_frame(&encoded, 1200),
            Err(DatagramFrameError::PayloadLengthMismatch { .. })
        ));
        Ok(())
    }
}
