-- Sealed secret fields (ADR 0004). Each row is one field of one version of
-- an entry (a credential, a break-glass account …). The vault binds the
-- ciphertext to exactly this owner, version and field: a row copied or moved
-- elsewhere cannot be opened. Plaintext never touches this table.
CREATE TABLE secret_fields (
    owner_id uuid NOT NULL,
    version integer NOT NULL CHECK (version > 0),
    field text NOT NULL,
    scheme text NOT NULL,
    kek_id text NOT NULL,
    kek_version integer NOT NULL,
    wrapped_key bytea NOT NULL,
    ciphertext bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_id, version, field)
);

-- Finds everything still wrapped with an older master key after rotation.
CREATE INDEX secret_fields_kek ON secret_fields (kek_id, kek_version);
