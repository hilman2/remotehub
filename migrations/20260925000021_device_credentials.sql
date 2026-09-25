-- Credentials of a device's own (#91, sign-in mode 'device'): user name and
-- domain here, the password sealed in secret_fields with the device as
-- owner, field 'password' and secret_version as version; 0: none set.
ALTER TABLE devices
    ADD COLUMN username text NOT NULL DEFAULT '',
    ADD COLUMN domain text NOT NULL DEFAULT '',
    ADD COLUMN secret_version integer NOT NULL DEFAULT 0 CHECK (secret_version >= 0);

ALTER TABLE devices DROP CONSTRAINT devices_auth_mode_check;
ALTER TABLE devices ADD CONSTRAINT devices_auth_mode_check
    CHECK (auth_mode IN ('stored', 'ask', 'own', 'certificate', 'laps', 'device'));
