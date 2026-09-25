-- The mail server (#145), set on the settings page and in the setup wizard.
-- One row at most; without it, remotehub sends no mail. A password, if the
-- server wants one, is sealed in secret_fields, with this row's id as owner
-- and password_version as version.
CREATE TABLE mail (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    singleton boolean NOT NULL DEFAULT true UNIQUE CHECK (singleton),
    host text NOT NULL,
    port integer NOT NULL CHECK (port BETWEEN 1 AND 65535),
    -- tls: TLS from the start; starttls: upgraded; none: an internal relay.
    security text NOT NULL CHECK (security IN ('tls', 'starttls', 'none')),
    username text,
    password_version integer CHECK (password_version > 0),
    -- A password never goes over an unencrypted connection.
    CHECK (security <> 'none' OR password_version IS NULL),
    from_address text NOT NULL,
    from_name text NOT NULL,
    -- PEM of CA certificates the server's certificate may come from, besides
    -- the system's.
    ca_pem text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid REFERENCES users (id)
);
