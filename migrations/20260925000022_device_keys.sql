-- A device's own credentials may be an SSH key as well (#91). As for
-- credentials: the kind, and what identifies a key, to show it; the key
-- itself, its passphrase and certificate are sealed in secret_fields.
ALTER TABLE devices
    ADD COLUMN secret_kind text NOT NULL DEFAULT 'password'
        CHECK (secret_kind IN ('password', 'ssh_key')),
    ADD COLUMN key_algorithm text,
    ADD COLUMN key_fingerprint text,
    ADD COLUMN has_certificate boolean NOT NULL DEFAULT false;
