-- The journal of a device (#90): every connection, with the purpose given
-- for it, and the notes people leave. Entries are only ever added.
CREATE TABLE device_journal (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users (id),
    kind text NOT NULL CHECK (kind IN ('connection', 'note')),
    -- The purpose of a connection (may be empty), or the note.
    text text NOT NULL DEFAULT '' CHECK (length(text) <= 2000),
    -- Connections only: the protocol, and when the session ended.
    protocol text,
    ended_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (kind = 'connection' OR (protocol IS NULL AND ended_at IS NULL AND length(trim(text)) > 0))
);

CREATE INDEX device_journal_device ON device_journal (device_id, created_at DESC);

-- Users and groups (by SID) who state a purpose before every connection.
CREATE TABLE purpose_principals (
    principal_sid text PRIMARY KEY,
    principal_kind text NOT NULL CHECK (principal_kind IN ('user', 'group')),
    -- Name when added, for display; the SID decides.
    principal_name text NOT NULL,
    created_by uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
