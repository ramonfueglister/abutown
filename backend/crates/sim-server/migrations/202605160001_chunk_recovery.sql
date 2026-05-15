ALTER TABLE world_events
  ADD COLUMN IF NOT EXISTS chunk_x INTEGER,
  ADD COLUMN IF NOT EXISTS chunk_y INTEGER,
  ADD COLUMN IF NOT EXISTS chunk_version BIGINT;

UPDATE world_events
   SET chunk_x = (payload->'coord'->>'x')::int,
       chunk_y = (payload->'coord'->>'y')::int,
       chunk_version = version
 WHERE chunk_x IS NULL;

ALTER TABLE world_events
  ALTER COLUMN chunk_x SET NOT NULL,
  ALTER COLUMN chunk_y SET NOT NULL,
  ALTER COLUMN chunk_version SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS world_events_world_command_uniq
  ON world_events (world_id, command_id);

CREATE INDEX IF NOT EXISTS world_events_chunk_version_idx
  ON world_events (world_id, chunk_x, chunk_y, chunk_version);
