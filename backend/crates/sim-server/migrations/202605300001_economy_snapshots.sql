CREATE TABLE IF NOT EXISTS economy_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    base_world_id TEXT,
    base_world_schema_version INTEGER,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS economy_snapshots_base_world_idx
  ON economy_snapshots (world_id, base_world_id, base_world_schema_version)
