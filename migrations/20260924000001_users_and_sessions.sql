-- People who have signed in. Directory users are identified by their SID
-- and GUID (ADR 0005); names are refreshed at every sign-in. Local users are
-- break-glass accounts (#9).
CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind text NOT NULL CHECK (kind IN ('directory', 'local')),
    sid text UNIQUE,
    guid uuid UNIQUE,
    username text NOT NULL,
    display_name text NOT NULL,
    email text,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_sign_in_at timestamptz,
    CHECK (kind <> 'directory' OR (sid IS NOT NULL AND guid IS NOT NULL))
);

-- Server-side sessions. Only the SHA-256 hash of the token is stored; the
-- token itself exists only in the user's cookie, so a copy of this table
-- opens no session. Group SIDs are taken at sign-in and hold for the session.
CREATE TABLE sessions (
    token_hash bytea PRIMARY KEY CHECK (length(token_hash) = 32),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    groups text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE INDEX sessions_expires_at ON sessions (expires_at);
