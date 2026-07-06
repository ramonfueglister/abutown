-- backend/crates/sim-server/migrations/202607060001_building_attributes.sql
-- Authoritative per-building enrichment: ÖREB Bauzone (allowed) + GWR (is).
-- Seeded from data/winterthur/building-attributes.json via
-- `sim-server load-building-attributes` (writes only through DATABASE_URL).
CREATE TABLE IF NOT EXISTS building_attributes (
  world_id      TEXT        NOT NULL,
  building_id   TEXT        NOT NULL,   -- swissBUILDINGS3D UUID
  egid          BIGINT,
  gwr_category  TEXT,
  gwr_class     TEXT,
  bauzone       TEXT,
  bauzone_code  TEXT,
  raw           JSONB       NOT NULL DEFAULT '{}'::jsonb,
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (world_id, building_id)
);
-- Open data, safe to read publicly; writes only via the direct Postgres
-- connection (sqlx), never via PostgREST/anon.
ALTER TABLE building_attributes ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS building_attributes_public_read ON building_attributes;
CREATE POLICY building_attributes_public_read ON building_attributes FOR SELECT USING (true);
