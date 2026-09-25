-- The organisation recovery key for personal vaults (#95, ADR 0009).

-- Public keys only (uncompressed P-256 points); the private key never
-- reaches the server. The newest one is the one vaults are wrapped for.
CREATE TABLE recovery_keys (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    public_key bytea NOT NULL CHECK (length(public_key) = 65),
    created_by_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- A vault wrapped for a recovery key is one more way to unlock it, of kind
-- `organisation`: its params name the key and hold the ephemeral public key.
ALTER TABLE personal_vault_unlocks DROP CONSTRAINT personal_vault_unlocks_kind_check;
ALTER TABLE personal_vault_unlocks ADD CONSTRAINT personal_vault_unlocks_kind_check
    CHECK (kind IN ('passkey', 'passphrase', 'recovery', 'organisation'));
DROP INDEX personal_vault_unlocks_single;
CREATE UNIQUE INDEX personal_vault_unlocks_single ON personal_vault_unlocks (user_id, kind)
    WHERE kind IN ('passphrase', 'recovery', 'organisation');

-- Asked for by an administrator, approved by a security officer who is
-- someone else, carried out in the requester's browser.
CREATE TABLE vault_recoveries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- `passphrase`: a one-time recovery key for the owner; `handover`: the
    -- entries go into a shared folder.
    kind text NOT NULL CHECK (kind IN ('passphrase', 'handover')),
    reason text NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    requester_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    requester_name text NOT NULL,
    approver_name text,
    approved_at timestamptz,
    completed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- One open recovery per vault.
CREATE UNIQUE INDEX vault_recoveries_open ON vault_recoveries (user_id) WHERE completed_at IS NULL;
