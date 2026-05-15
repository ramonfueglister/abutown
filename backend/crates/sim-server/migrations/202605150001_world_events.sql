CREATE TABLE IF NOT EXISTS world_events (
    event_id TEXT PRIMARY KEY,
    world_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    version BIGINT NOT NULL CHECK (version >= 0),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS world_events_world_version_idx
    ON world_events (world_id, version);

CREATE INDEX IF NOT EXISTS world_events_world_tick_idx
    ON world_events (world_id, tick);

CREATE INDEX IF NOT EXISTS world_events_world_command_idx
    ON world_events (world_id, command_id);
