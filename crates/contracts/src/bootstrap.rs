use thiserror::Error;

use crate::MessageKind;

const MAGIC: [u8; 4] = *b"AUR2";
pub const FRAME_HEADER_BYTES: usize = 8;
pub const CLIENT_HELLO_BYTES: usize = 24;
pub const SERVER_HELLO_BYTES: usize = 62;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BootstrapCodecError {
    #[error("invalid Aurenfall bootstrap magic")]
    InvalidMagic,
    #[error("unknown message kind {0}")]
    UnknownMessageKind(u16),
    #[error("invalid payload length: expected {expected}, got {actual}")]
    InvalidPayloadLength { expected: usize, actual: usize },
    #[error("invalid boolean byte {0}")]
    InvalidBoolean(u8),
    #[error("unknown handshake reject code {0}")]
    UnknownRejectCode(u8),
    #[error("payload too large for bootstrap frame")]
    PayloadTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub kind: MessageKind,
    pub payload_len: u16,
}

impl FrameHeader {
    pub fn new(kind: MessageKind, payload_len: usize) -> Result<Self, BootstrapCodecError> {
        let payload_len = u16::try_from(payload_len).map_err(|_| BootstrapCodecError::PayloadTooLarge)?;
        Ok(Self { kind, payload_len })
    }

    #[must_use]
    pub fn encode(self) -> [u8; FRAME_HEADER_BYTES] {
        let mut bytes = [0_u8; FRAME_HEADER_BYTES];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4..6].copy_from_slice(&(self.kind as u16).to_be_bytes());
        bytes[6..8].copy_from_slice(&self.payload_len.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; FRAME_HEADER_BYTES]) -> Result<Self, BootstrapCodecError> {
        if bytes[0..4] != MAGIC {
            return Err(BootstrapCodecError::InvalidMagic);
        }
        let kind = MessageKind::try_from(u16::from_be_bytes([bytes[4], bytes[5]]))?;
        let payload_len = u16::from_be_bytes([bytes[6], bytes[7]]);
        Ok(Self { kind, payload_len })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientHello {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub client_build: u32,
    pub client_nonce: [u8; 16],
}

impl ClientHello {
    #[must_use]
    pub fn encode(self) -> [u8; CLIENT_HELLO_BYTES] {
        let mut bytes = [0_u8; CLIENT_HELLO_BYTES];
        bytes[0..2].copy_from_slice(&self.protocol_major.to_be_bytes());
        bytes[2..4].copy_from_slice(&self.protocol_minor.to_be_bytes());
        bytes[4..8].copy_from_slice(&self.client_build.to_be_bytes());
        bytes[8..24].copy_from_slice(&self.client_nonce);
        bytes
    }

    pub fn decode(payload: &[u8]) -> Result<Self, BootstrapCodecError> {
        if payload.len() != CLIENT_HELLO_BYTES {
            return Err(BootstrapCodecError::InvalidPayloadLength {
                expected: CLIENT_HELLO_BYTES,
                actual: payload.len(),
            });
        }
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&payload[8..24]);
        Ok(Self {
            protocol_major: u16::from_be_bytes([payload[0], payload[1]]),
            protocol_minor: u16::from_be_bytes([payload[2], payload[3]]),
            client_build: u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
            client_nonce: nonce,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HandshakeRejectCode {
    None = 0,
    IncompatibleProtocol = 1,
    ClientBuildTooOld = 2,
}

impl TryFrom<u8> for HandshakeRejectCode {
    type Error = BootstrapCodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::IncompatibleProtocol),
            2 => Ok(Self::ClientBuildTooOld),
            _ => Err(BootstrapCodecError::UnknownRejectCode(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerHello {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub accepted: bool,
    pub reject_code: HandshakeRejectCode,
    pub connection_id: u64,
    pub session_id: u64,
    pub universe_id: u64,
    pub client_nonce_echo: [u8; 16],
    pub server_nonce: [u8; 16],
}

impl ServerHello {
    #[must_use]
    pub fn encode(self) -> [u8; SERVER_HELLO_BYTES] {
        let mut bytes = [0_u8; SERVER_HELLO_BYTES];
        bytes[0..2].copy_from_slice(&self.protocol_major.to_be_bytes());
        bytes[2..4].copy_from_slice(&self.protocol_minor.to_be_bytes());
        bytes[4] = u8::from(self.accepted);
        bytes[5] = self.reject_code as u8;
        bytes[6..14].copy_from_slice(&self.connection_id.to_be_bytes());
        bytes[14..22].copy_from_slice(&self.session_id.to_be_bytes());
        bytes[22..30].copy_from_slice(&self.universe_id.to_be_bytes());
        bytes[30..46].copy_from_slice(&self.client_nonce_echo);
        bytes[46..62].copy_from_slice(&self.server_nonce);
        bytes
    }

    pub fn decode(payload: &[u8]) -> Result<Self, BootstrapCodecError> {
        if payload.len() != SERVER_HELLO_BYTES {
            return Err(BootstrapCodecError::InvalidPayloadLength {
                expected: SERVER_HELLO_BYTES,
                actual: payload.len(),
            });
        }
        let accepted = match payload[4] {
            0 => false,
            1 => true,
            value => return Err(BootstrapCodecError::InvalidBoolean(value)),
        };
        let mut client_nonce_echo = [0_u8; 16];
        client_nonce_echo.copy_from_slice(&payload[30..46]);
        let mut server_nonce = [0_u8; 16];
        server_nonce.copy_from_slice(&payload[46..62]);
        Ok(Self {
            protocol_major: u16::from_be_bytes([payload[0], payload[1]]),
            protocol_minor: u16::from_be_bytes([payload[2], payload[3]]),
            accepted,
            reject_code: HandshakeRejectCode::try_from(payload[5])?,
            connection_id: u64::from_be_bytes([
                payload[6],
                payload[7],
                payload[8],
                payload[9],
                payload[10],
                payload[11],
                payload[12],
                payload[13],
            ]),
            session_id: u64::from_be_bytes([
                payload[14],
                payload[15],
                payload[16],
                payload[17],
                payload[18],
                payload[19],
                payload[20],
                payload[21],
            ]),
            universe_id: u64::from_be_bytes([
                payload[22],
                payload[23],
                payload[24],
                payload[25],
                payload[26],
                payload[27],
                payload[28],
                payload[29],
            ]),
            client_nonce_echo,
            server_nonce,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_round_trips() {
        let hello = ClientHello {
            protocol_major: 1,
            protocol_minor: 2,
            client_build: 42,
            client_nonce: [7; 16],
        };
        assert_eq!(ClientHello::decode(&hello.encode()), Ok(hello));
    }

    #[test]
    fn server_hello_round_trips() {
        let hello = ServerHello {
            protocol_major: 1,
            protocol_minor: 0,
            accepted: true,
            reject_code: HandshakeRejectCode::None,
            connection_id: 11,
            session_id: 12,
            universe_id: 13,
            client_nonce_echo: [3; 16],
            server_nonce: [4; 16],
        };
        assert_eq!(ServerHello::decode(&hello.encode()), Ok(hello));
    }

    #[test]
    fn frame_header_rejects_wrong_magic() {
        let mut bytes = FrameHeader {
            kind: MessageKind::ClientHello,
            payload_len: 24,
        }
        .encode();
        bytes[0] = b'X';
        assert_eq!(FrameHeader::decode(bytes), Err(BootstrapCodecError::InvalidMagic));
    }
}
