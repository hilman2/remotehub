-- The master keys, sealed for the organisation recovery key (#96, ADR 0009):
-- with a database backup and the recovery key's private key,
-- `remotehub recover-master-key` restores the master key file.
CREATE TABLE master_key_escrow (
    recovery_key_id uuid NOT NULL REFERENCES recovery_keys (id) ON DELETE CASCADE,
    kek_id text NOT NULL,
    kek_version integer NOT NULL,
    -- The public half of the key pair made for this seal alone.
    ephemeral bytea NOT NULL CHECK (length(ephemeral) = 65),
    sealed bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (recovery_key_id, kek_id, kek_version)
);
