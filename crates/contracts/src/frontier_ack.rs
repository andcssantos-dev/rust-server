use thiserror::Error;

pub const FRONTIER_MANIFEST_ACK_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontierManifestAck {
    pub revision: u64,
}

impl FrontierManifestAck {
    pub fn new(revision: u64) -> Result<Self, FrontierManifestAckCodecError> {
        if revision == 0 {
            return Err(FrontierManifestAckCodecError::ZeroRevision);
        }
        Ok(Self { revision })
    }

    #[must_use]
    pub fn encode(self) -> [u8; FRONTIER_MANIFEST_ACK_BYTES] {
        self.revision.to_le_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, FrontierManifestAckCodecError> {
        if payload.len() != FRONTIER_MANIFEST_ACK_BYTES {
            return Err(FrontierManifestAckCodecError::InvalidLength {
                expected: FRONTIER_MANIFEST_ACK_BYTES,
                actual: payload.len(),
            });
        }
        let revision = u64::from_le_bytes(payload.try_into().map_err(|_| {
            FrontierManifestAckCodecError::InvalidLength {
                expected: FRONTIER_MANIFEST_ACK_BYTES,
                actual: payload.len(),
            }
        })?);
        Self::new(revision)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FrontierManifestAckCodecError {
    #[error("frontier manifest ack must contain {expected} bytes, got {actual}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("frontier manifest ack revision must be non-zero")]
    ZeroRevision,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontier_manifest_ack_roundtrips_applied_revision() -> Result<(), FrontierManifestAckCodecError> {
        let ack = FrontierManifestAck::new(42)?;
        assert_eq!(FrontierManifestAck::decode(&ack.encode())?, ack);
        Ok(())
    }

    #[test]
    fn frontier_manifest_ack_rejects_zero_and_wrong_length() {
        assert_eq!(
            FrontierManifestAck::new(0),
            Err(FrontierManifestAckCodecError::ZeroRevision)
        );
        assert!(matches!(
            FrontierManifestAck::decode(&[1, 2, 3]),
            Err(FrontierManifestAckCodecError::InvalidLength {
                expected: FRONTIER_MANIFEST_ACK_BYTES,
                actual: 3,
            })
        ));
        assert_eq!(
            FrontierManifestAck::decode(&0_u64.to_le_bytes()),
            Err(FrontierManifestAckCodecError::ZeroRevision)
        );
    }
}
