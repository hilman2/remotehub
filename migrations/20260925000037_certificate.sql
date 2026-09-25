-- A TLS certificate of your own for Caddy of the ops package (#146). One row
-- at most; without it, Caddy gets one from Let's Encrypt or its own CA. The
-- key is sealed in secret_fields, with this row's id as owner: a new
-- certificate is a new row.
CREATE TABLE certificate (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    singleton boolean NOT NULL DEFAULT true UNIQUE CHECK (singleton),
    -- The chain as PEM, the server's own certificate first.
    chain_pem text NOT NULL,
    not_after timestamptz NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid REFERENCES users (id)
);
