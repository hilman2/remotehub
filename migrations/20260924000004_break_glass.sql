-- Break-glass accounts (ADR 0005). Their TOTP secret is sealed in
-- secret_fields (owner = user, field 'totp', version = totp_version).
CREATE TABLE local_accounts (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    password_hash text NOT NULL,
    totp_version integer NOT NULL DEFAULT 1,
    -- Last TOTP time step used; a code is only accepted once.
    last_totp_step bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Local user names are unique, whatever their case.
CREATE UNIQUE INDEX users_local_username ON users (lower(username)) WHERE kind = 'local';

-- Deleted break-glass users stay as rows, because the audit log refers to them.
ALTER TABLE users DROP CONSTRAINT users_kind_check;
ALTER TABLE users ADD CONSTRAINT users_kind_check CHECK (kind IN ('directory', 'local', 'deleted'));
