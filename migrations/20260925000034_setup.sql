-- The setup wizard (#143). A fresh installation has no administrator: the
-- installer prints a link with a one-time code (`remotehub setup-code`), and
-- the wizard creates the first administrator with it. Until then, the API
-- serves only the wizard.
ALTER TABLE instance
    -- SHA-256 of the setup code; none once it is spent or setup is done.
    ADD COLUMN setup_code_hash bytea CHECK (length(setup_code_hash) = 32),
    -- The local account the wizard created as the first administrator.
    ADD COLUMN setup_administrator uuid REFERENCES users (id) ON DELETE SET NULL,
    -- The last wizard step done: the wizard continues after it.
    ADD COLUMN setup_step smallint NOT NULL DEFAULT 0,
    -- Once set, setup never opens again.
    ADD COLUMN setup_completed_at timestamptz;
