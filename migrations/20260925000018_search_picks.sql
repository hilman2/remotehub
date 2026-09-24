-- What people picked after searching (#81): the device list sorts by it.
-- One row per user, item and query; the browser ranks, the server keeps.
CREATE TABLE search_picks (
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- `folder:<id>`, `device:<id>` or `credential:<id>`.
    key text NOT NULL CHECK (key ~ '^(folder|device|credential):[0-9a-f-]{36}$'),
    -- Normalized as the browser searched it; empty for a pick without a query.
    query text NOT NULL CHECK (length(query) <= 100),
    count integer NOT NULL DEFAULT 1 CHECK (count > 0),
    last_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, key, query)
);

-- The same for the personal vault, encrypted by the browser like its
-- entries: the server must not learn what someone looks for there.
CREATE TABLE personal_search (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    nonce bytea NOT NULL CHECK (length(nonce) = 12),
    ciphertext bytea NOT NULL CHECK (length(ciphertext) BETWEEN 16 AND 65536),
    updated_at timestamptz NOT NULL DEFAULT now()
);
