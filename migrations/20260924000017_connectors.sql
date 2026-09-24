-- Site connectors (#19, ADR 0008): a device behind one is reached through
-- the tunnel its connector keeps open to remotehub.
CREATE TABLE connectors (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL UNIQUE CHECK (length(trim(name)) BETWEEN 1 AND 200),
    -- SHA-256 of the token the connector signs in with; the token itself is
    -- shown once and kept nowhere.
    token_hash bytea NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz
);

-- A connector with devices cannot be deleted: they would silently be
-- reached directly, possibly a different machine of the same address.
ALTER TABLE devices ADD COLUMN connector_id uuid REFERENCES connectors (id) ON DELETE RESTRICT;
CREATE INDEX devices_connector ON devices (connector_id);
