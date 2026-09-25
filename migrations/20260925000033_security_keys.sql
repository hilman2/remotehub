-- Security keys and passkeys as the second factor of directory users
-- (#129, crates/server/src/webauthn.rs), next to the authenticator app of
-- `second_factors`.
CREATE TABLE security_keys (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    credential_id bytea NOT NULL UNIQUE CHECK (length(credential_id) BETWEEN 1 AND 1023),
    -- An uncompressed P-256 point: only ES256 keys are taken.
    public_key bytea NOT NULL CHECK (length(public_key) = 65),
    -- The key's signature counter; one that goes backwards is a copy.
    sign_count bigint NOT NULL DEFAULT 0,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);

CREATE INDEX security_keys_user ON security_keys (user_id);

-- Challenges for registering a key or signing in with one: each is used
-- once and only until it expires. No foreign key on the user: a sign-in
-- creates its challenge while its own transaction holds the user's row,
-- and the key's check would wait for that transaction forever.
CREATE TABLE webauthn_challenges (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL,
    purpose text NOT NULL CHECK (purpose IN ('register', 'sign_in')),
    challenge bytea NOT NULL CHECK (length(challenge) = 32),
    expires_at timestamptz NOT NULL
);
