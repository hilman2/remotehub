-- Files kept with a shared credential (#100). The content is sealed in
-- secret_fields with the attachment as owner, version 1, field 'content'.
CREATE TABLE credential_attachments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    credential_id uuid NOT NULL REFERENCES credentials (id) ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 255),
    media_type text NOT NULL CHECK (length(media_type) BETWEEN 1 AND 255),
    size integer NOT NULL CHECK (size BETWEEN 0 AND 5242880),
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users (id),
    UNIQUE (credential_id, name)
);

-- Files of personal vault entries, sealed in the browser like the entries
-- (e2e_user_v1); the entry lists them by ID.
CREATE TABLE personal_attachments (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    nonce bytea NOT NULL CHECK (length(nonce) = 12),
    ciphertext bytea NOT NULL CHECK (length(ciphertext) BETWEEN 16 AND 5242896),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX personal_attachments_user ON personal_attachments (user_id);
