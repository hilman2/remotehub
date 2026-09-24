-- The folder tree with devices, credentials and grants (ADR 0005).
-- Permissions are decided only by authorize() (crates/model), never in SQL.

CREATE TABLE folders (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- NULL: at the top level.
    parent_id uuid REFERENCES folders (id) ON DELETE RESTRICT,
    name text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (parent_id IS DISTINCT FROM id),
    UNIQUE NULLS NOT DISTINCT (parent_id, name)
);

-- Credentials: name, user name and domain are searchable plain text; the
-- password is sealed in secret_fields (owner = credential, field 'password',
-- version = version). Every change of the password makes a new version.
CREATE TABLE credentials (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id uuid NOT NULL REFERENCES folders (id) ON DELETE RESTRICT,
    name text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    username text NOT NULL DEFAULT '',
    domain text NOT NULL DEFAULT '',
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (folder_id, name)
);

CREATE TABLE devices (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id uuid NOT NULL REFERENCES folders (id) ON DELETE RESTRICT,
    name text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    protocol text NOT NULL CHECK (protocol IN ('ssh', 'rdp', 'vnc')),
    host text NOT NULL CHECK (length(trim(host)) BETWEEN 1 AND 253),
    port integer NOT NULL CHECK (port BETWEEN 1 AND 65535),
    -- How to authenticate when connecting: a stored credential, ask every
    -- time, or the user's own directory account.
    auth_mode text NOT NULL DEFAULT 'ask' CHECK (auth_mode IN ('stored', 'ask', 'own')),
    credential_id uuid REFERENCES credentials (id) ON DELETE SET NULL,
    description text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (folder_id, name)
);

CREATE INDEX devices_credential ON devices (credential_id);

-- A role for a principal (SID of a user or group) on exactly one object.
CREATE TABLE grants (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id uuid REFERENCES folders (id) ON DELETE CASCADE,
    device_id uuid REFERENCES devices (id) ON DELETE CASCADE,
    credential_id uuid REFERENCES credentials (id) ON DELETE CASCADE,
    principal_kind text NOT NULL CHECK (principal_kind IN ('user', 'group')),
    principal_sid text NOT NULL,
    -- Name at the time of granting, for display; the SID decides.
    principal_name text NOT NULL,
    role text NOT NULL CHECK (role IN ('list', 'connect', 'reveal', 'edit', 'manage')),
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users (id),
    CHECK (num_nonnulls(folder_id, device_id, credential_id) = 1),
    UNIQUE NULLS NOT DISTINCT (folder_id, device_id, credential_id, principal_sid)
);
