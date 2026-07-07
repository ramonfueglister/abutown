-- Compress the world-core snapshot payload: store zstd-compressed JSON bytes in
-- a new BYTEA column instead of the ~1-2.5 MB uncompressed JSONB, cutting the
-- Supabase write bandwidth ~10x on top of the coarser persist cadence.
--
-- No-Wipe: the existing `payload JSONB` column is kept and its NOT NULL is
-- dropped so a compressed write can leave it NULL. Old rows written before this
-- migration still carry their JSONB in `payload` and remain readable — the read
-- path prefers `payload_z` when present and falls back to `payload` otherwise.
ALTER TABLE world_core_snapshots ADD COLUMN IF NOT EXISTS payload_z BYTEA;
ALTER TABLE world_core_snapshots ALTER COLUMN payload DROP NOT NULL;
