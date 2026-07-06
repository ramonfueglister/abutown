CREATE TABLE IF NOT EXISTS world_core_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    schema_version INTEGER NOT NULL,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
