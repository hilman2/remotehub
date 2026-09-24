-- The personal vault (#22): entries only their owner can read, encrypted in
-- the browser (scheme e2e_user_v1, ADR 0004). The server stores ciphertext
-- and wrapped keys and can decrypt neither.

-- The vault key, wrapped once per way to unlock it: a passkey (WebAuthn
-- PRF), a passphrase or the recovery key.
CREATE TABLE personal_vault_unlocks (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('passkey', 'passphrase', 'recovery')),
    -- What the browser needs to derive the wrapping key again: the passkey's
    -- credential ID and PRF salt, or the passphrase's salt and iterations.
    params jsonb NOT NULL,
    wrapped_key bytea NOT NULL CHECK (length(wrapped_key) BETWEEN 16 AND 256),
    label text NOT NULL DEFAULT '' CHECK (length(label) <= 100),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX personal_vault_unlocks_user ON personal_vault_unlocks (user_id);
-- One passphrase and one recovery key per vault; passkeys as many as needed.
CREATE UNIQUE INDEX personal_vault_unlocks_single ON personal_vault_unlocks (user_id, kind)
    WHERE kind IN ('passphrase', 'recovery');

CREATE TABLE personal_entries (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    scheme text NOT NULL CHECK (scheme = 'e2e_user_v1'),
    nonce bytea NOT NULL CHECK (length(nonce) = 12),
    ciphertext bytea NOT NULL CHECK (length(ciphertext) BETWEEN 16 AND 65536),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX personal_entries_user ON personal_entries (user_id);
