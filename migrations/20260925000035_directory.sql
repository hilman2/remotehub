-- The directory connection (#144), set on the settings page and in the setup
-- wizard. One row at most; without it, only local and break-glass accounts
-- sign in. The service account's password is sealed in secret_fields, with
-- this row's id as owner and password_version as version.
CREATE TABLE directory (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    singleton boolean NOT NULL DEFAULT true UNIQUE CHECK (singleton),
    url text NOT NULL,
    starttls boolean NOT NULL,
    -- PEM: CA certificates, and server certificates trusted as they are.
    ca_pem text,
    bind_dn text NOT NULL,
    password_version integer NOT NULL CHECK (password_version > 0),
    base_dn text NOT NULL,
    user_filter text,
    timeout_seconds integer NOT NULL CHECK (timeout_seconds BETWEEN 1 AND 60),
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid REFERENCES users (id)
);
