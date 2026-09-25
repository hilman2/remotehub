-- A second factor for directory sign-ins (#107): an authenticator app whose
-- secret is sealed in secret_fields (owner = user, field 'totp').
CREATE TABLE second_factors (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    totp_version integer NOT NULL,
    -- Each code works once: the time step of the last one used.
    last_totp_step bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Users and groups who set one up before their next session, named like the
-- principal of a grant.
CREATE TABLE second_factor_principals (
    principal_sid text PRIMARY KEY,
    principal_kind text NOT NULL CHECK (principal_kind IN ('user', 'group')),
    principal_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users (id)
);
