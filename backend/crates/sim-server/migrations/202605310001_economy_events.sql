CREATE TABLE IF NOT EXISTS economy_events (
    id BIGSERIAL PRIMARY KEY,
    world_id TEXT NOT NULL,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS economy_events_world_id_idx
  ON economy_events (world_id, id);
CREATE INDEX IF NOT EXISTS economy_events_world_tick_idx
  ON economy_events (world_id, tick)
