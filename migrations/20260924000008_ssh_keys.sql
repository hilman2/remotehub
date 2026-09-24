-- Credentials are passwords or SSH keys (ADR 0004). For keys, the private
-- key, its passphrase and an optional OpenSSH user certificate are sealed in
-- secret_fields ('private_key', 'passphrase', 'certificate'); only what
-- identifies the key is plain, to show it without revealing it.
ALTER TABLE credentials
    ADD COLUMN kind text NOT NULL DEFAULT 'password' CHECK (kind IN ('password', 'ssh_key')),
    ADD COLUMN key_algorithm text,
    ADD COLUMN key_fingerprint text,
    ADD COLUMN has_certificate boolean NOT NULL DEFAULT false;
