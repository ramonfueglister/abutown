CREATE TABLE IF NOT EXISTS chunk_snapshots (
    world_id TEXT NOT NULL,
    chunk_x INTEGER NOT NULL,
    chunk_y INTEGER NOT NULL,
    chunk_state TEXT NOT NULL,
    chunk_version BIGINT NOT NULL CHECK (chunk_version >= 0),
    tile_count INTEGER NOT NULL CHECK (tile_count >= 0),
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (world_id, chunk_x, chunk_y)
);

CREATE INDEX IF NOT EXISTS chunk_snapshots_world_updated_idx
    ON chunk_snapshots (world_id, updated_at DESC);
