-- What a KeePass entry has besides user name and password (#98).
ALTER TABLE credentials
    ADD COLUMN url text NOT NULL DEFAULT '' CHECK (length(url) <= 2000),
    ADD COLUMN notes text NOT NULL DEFAULT '' CHECK (length(notes) <= 10000),
    -- One of KeePass' standard icons, 0 to 68 (0: the key).
    ADD COLUMN icon smallint NOT NULL DEFAULT 0 CHECK (icon BETWEEN 0 AND 68),
    -- Custom fields in order, [{"name", "value"}] or [{"name", "protected": true}].
    -- A protected field's value is sealed in secret_fields as 'field:<name>',
    -- with the credential's version like the password.
    ADD COLUMN fields jsonb NOT NULL DEFAULT '[]';
