ALTER TABLE chunk_snapshots
  ADD COLUMN IF NOT EXISTS base_world_id TEXT,
  ADD COLUMN IF NOT EXISTS base_world_schema_version INTEGER;

ALTER TABLE mobility_snapshots
  ADD COLUMN IF NOT EXISTS base_world_id TEXT,
  ADD COLUMN IF NOT EXISTS base_world_schema_version INTEGER;

CREATE INDEX IF NOT EXISTS chunk_snapshots_base_world_idx
  ON chunk_snapshots (world_id, base_world_id, base_world_schema_version, chunk_x, chunk_y);

CREATE INDEX IF NOT EXISTS mobility_snapshots_base_world_idx
  ON mobility_snapshots (world_id, base_world_id, base_world_schema_version);
