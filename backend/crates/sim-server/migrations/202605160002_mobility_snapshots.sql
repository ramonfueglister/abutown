CREATE TABLE IF NOT EXISTS mobility_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
