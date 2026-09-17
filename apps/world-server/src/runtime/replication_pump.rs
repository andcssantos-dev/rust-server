use aurenfall_contracts::SelfMovementSnapshotV2;

pub struct ReplicatedSnapshot {
    pub connection_id: u64,
    pub payload: Vec<u8>,
}

pub fn encode_movement_snapshot(
    connection_id: u64,
    snapshot: &SelfMovementSnapshotV2,
) -> ReplicatedSnapshot {
    let payload = snapshot.encode_wire().to_vec();
    ReplicatedSnapshot {
        connection_id,
        payload,
    }
}