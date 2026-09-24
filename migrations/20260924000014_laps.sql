-- SSH and RDP devices can sign in as the local administrator whose password
-- LAPS keeps in the directory, read at connection time (#18).
ALTER TABLE devices DROP CONSTRAINT devices_auth_mode_check;
ALTER TABLE devices ADD CONSTRAINT devices_auth_mode_check
    CHECK (auth_mode IN ('stored', 'ask', 'own', 'certificate', 'laps'));
