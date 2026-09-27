-- The browser extension (#201, ADR 0017). Its sessions live beside those of
-- the browser, told apart by `client`, so the idle time, the maximum lifetime
-- and directory changes (#108) end them alike. `id` names a session on
-- *My account* without showing its token hash; `name` is the browser the
-- user connected, as the connect page described it.
ALTER TABLE sessions
    ADD COLUMN id uuid NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    ADD COLUMN client text NOT NULL DEFAULT 'web' CHECK (client IN ('web', 'extension')),
    ADD COLUMN name text,
    ADD CHECK (client = 'web' OR name IS NOT NULL);

-- Codes that hand the user of a signed-in browser to the extension: the
-- authorization code of OAuth 2.0 with PKCE (RFC 7636). The extension trades
-- the code and the verifier behind `challenge` for a session of its own.
-- Only the code's hash is kept; it holds for a minute and once. `groups` are
-- those of the browser session, as a new session takes them at sign-in.
CREATE TABLE extension_codes (
    code_hash bytea PRIMARY KEY CHECK (length(code_hash) = 32),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    groups text[] NOT NULL,
    extension_id text NOT NULL,
    challenge text NOT NULL,
    name text NOT NULL,
    expires_at timestamptz NOT NULL
);

CREATE INDEX extension_codes_expires_at ON extension_codes (expires_at);
