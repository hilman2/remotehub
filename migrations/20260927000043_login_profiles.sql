-- Devices and vault apart (#192): a device signs in with its own
-- credentials or a login profile, never with a vault entry.

-- A login profile is a named login many devices share. It lies in a device
-- folder and follows its grants; without a folder it is for administrators
-- only. Its secrets are sealed in secret_fields with the profile as owner
-- and secret_version as version, as a device's own credentials are.
CREATE TABLE login_profiles (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id uuid REFERENCES folders (id) ON DELETE RESTRICT,
    name text NOT NULL,
    username text NOT NULL DEFAULT '',
    domain text NOT NULL DEFAULT '',
    secret_kind text NOT NULL DEFAULT 'password' CHECK (secret_kind IN ('password', 'ssh_key')),
    secret_version integer NOT NULL CHECK (secret_version > 0),
    key_algorithm text,
    key_fingerprint text,
    has_certificate boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (folder_id, name)
);

-- Whether `folder` is `ancestor` or lies below it. A device may use only a
-- profile in its folder or one above.
CREATE FUNCTION folder_within(folder uuid, ancestor uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT id, parent_id FROM folders WHERE id = folder
        UNION
        SELECT f.id, f.parent_id FROM folders f JOIN up ON f.id = up.parent_id
    )
    SELECT EXISTS (SELECT 1 FROM up WHERE id = ancestor)
$$;

-- Nothing is live yet (#192): devices that used a vault entry ask instead.
UPDATE devices SET auth_mode = 'ask' WHERE auth_mode = 'stored';
ALTER TABLE devices DROP COLUMN credential_id;
ALTER TABLE devices ADD COLUMN profile_id uuid REFERENCES login_profiles (id) ON DELETE RESTRICT;
CREATE INDEX devices_profile ON devices (profile_id);
ALTER TABLE devices DROP CONSTRAINT devices_auth_mode_check;
ALTER TABLE devices ADD CONSTRAINT devices_auth_mode_check
    CHECK (auth_mode IN ('profile', 'ask', 'own', 'certificate', 'laps', 'device'));
ALTER TABLE devices ADD CONSTRAINT devices_profile_check
    CHECK ((auth_mode = 'profile') = (profile_id IS NOT NULL));

-- Vault entries are passwords; SSH keys for devices live in login profiles.
DELETE FROM secret_fields
WHERE owner_id IN (SELECT a.id FROM credential_attachments a
                   JOIN credentials c ON c.id = a.credential_id WHERE c.kind = 'ssh_key')
   OR owner_id IN (SELECT id FROM credentials WHERE kind = 'ssh_key');
DELETE FROM credentials WHERE kind = 'ssh_key';
ALTER TABLE credentials
    DROP COLUMN kind,
    DROP COLUMN key_algorithm,
    DROP COLUMN key_fingerprint,
    DROP COLUMN has_certificate;
