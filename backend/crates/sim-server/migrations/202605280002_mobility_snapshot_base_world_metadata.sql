ALTER TABLE mobility_snapshots
  ADD COLUMN IF NOT EXISTS base_world_id TEXT,
  ADD COLUMN IF NOT EXISTS base_world_schema_version INTEGER;

CREATE INDEX IF NOT EXISTS mobility_snapshots_base_world_idx
  ON mobility_snapshots (world_id, base_world_id, base_world_schema_version);
