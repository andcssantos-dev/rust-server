use aurenfall_contracts::{BootstrapCodecError, FRAME_HEADER_BYTES, FrameHeader, MessageKind};
use quinn::{ClosedStream, ReadExactError, RecvStream, SendStream, WriteError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FrameIoError {
    #[error(transparent)]
    Codec(#[from] BootstrapCodecError),
    #[error(transparent)]
    Read(#[from] ReadExactError),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error(transparent)]
    Finish(#[from] ClosedStream),
    #[error("frame payload {actual} exceeds configured maximum {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },
}

pub async fn read_single_frame(
    recv: &mut RecvStream,
    maximum_payload_bytes: usize,
) -> Result<(MessageKind, Vec<u8>), FrameIoError> {
    let mut header_bytes = [0_u8; FRAME_HEADER_BYTES];
    recv.read_exact(&mut header_bytes).await?;
    let header = FrameHeader::decode(header_bytes)?;
    let payload_len = usize::from(header.payload_len);
    if payload_len > maximum_payload_bytes {
        return Err(FrameIoError::PayloadTooLarge {
            actual: payload_len,
            maximum: maximum_payload_bytes,
        });
    }

    let mut payload = vec![0_u8; payload_len];
    recv.read_exact(&mut payload).await?;
    Ok((header.kind, payload))
}

pub async fn write_single_frame(
    send: &mut SendStream,
    kind: MessageKind,
    payload: &[u8],
    maximum_payload_bytes: usize,
) -> Result<(), FrameIoError> {
    if payload.len() > maximum_payload_bytes {
        return Err(FrameIoError::PayloadTooLarge {
            actual: payload.len(),
            maximum: maximum_payload_bytes,
        });
    }

    let header = FrameHeader::new(kind, payload.len())?.encode();
    send.write_all(&header).await?;
    send.write_all(payload).await?;
    send.finish()?;
    Ok(())
}
