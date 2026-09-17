CREATE TABLE IF NOT EXISTS character_inventories (
    character_id BIGINT PRIMARY KEY,
    inventory_data BYTEA NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);