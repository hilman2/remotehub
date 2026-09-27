-- The vault like KeePass (#193).
--
-- tags:       free words to find an entry by.
-- expires_on: the day the entry's password runs out; none: never.
-- has_totp:   whether a one-time password is sealed with it, as field
--             'totp' of the current version (the otpauth:// link).
-- deleted_at: in the recycle bin since then; none: in its collection.
--
-- Vault entries have no domain: KeePass keeps "DOMAIN\user" in the user
-- name, and so does remotehub now. Nothing is live, nothing to carry over.
ALTER TABLE credentials
    DROP COLUMN domain,
    ADD COLUMN tags text[] NOT NULL DEFAULT '{}',
    ADD COLUMN expires_on date,
    ADD COLUMN has_totp boolean NOT NULL DEFAULT false,
    ADD COLUMN deleted_at timestamptz;
