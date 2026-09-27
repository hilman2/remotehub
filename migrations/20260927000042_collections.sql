-- Shared credentials live in collections of their own, apart from the
-- device folders (#190). A collection is a tree like the folders, with
-- grants inherited downwards; authorize() decides as for folders.

CREATE TABLE collections (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- NULL: at the top level.
    parent_id uuid REFERENCES collections (id) ON DELETE RESTRICT,
    name text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (parent_id IS DISTINCT FROM id),
    UNIQUE NULLS NOT DISTINCT (parent_id, name)
);

ALTER TABLE grants ADD COLUMN collection_id uuid REFERENCES collections (id) ON DELETE CASCADE;
ALTER TABLE grants DROP CONSTRAINT grants_check;
ALTER TABLE grants ADD CONSTRAINT grants_one_object
    CHECK (num_nonnulls(folder_id, device_id, credential_id, collection_id) = 1);
DROP INDEX grants_permanent;
CREATE UNIQUE INDEX grants_permanent
    ON grants (folder_id, device_id, credential_id, collection_id, principal_sid)
    NULLS NOT DISTINCT WHERE expires_at IS NULL;

-- Every folder that holds credentials, and the folders above it, becomes a
-- collection of the same name and id, with the folder's grants: whoever
-- reached a credential before reaches it now.
WITH RECURSIVE holding AS (
    SELECT DISTINCT folder_id AS id FROM credentials
    UNION
    SELECT f.parent_id FROM folders f JOIN holding h ON f.id = h.id WHERE f.parent_id IS NOT NULL
)
INSERT INTO collections (id, parent_id, name, created_at, updated_at)
SELECT f.id, f.parent_id, f.name, f.created_at, f.updated_at
FROM folders f WHERE f.id IN (SELECT id FROM holding);

INSERT INTO grants (collection_id, principal_kind, principal_sid, principal_name, role, created_at,
                    created_by, expires_at, request_id)
SELECT folder_id, principal_kind, principal_sid, principal_name, role, created_at,
       created_by, expires_at, request_id
FROM grants WHERE folder_id IN (SELECT id FROM collections);

ALTER TABLE credentials ADD COLUMN collection_id uuid REFERENCES collections (id) ON DELETE RESTRICT;
UPDATE credentials SET collection_id = folder_id;
ALTER TABLE credentials ALTER COLUMN collection_id SET NOT NULL;
ALTER TABLE credentials DROP COLUMN folder_id;
ALTER TABLE credentials ADD CONSTRAINT credentials_collection_name UNIQUE (collection_id, name);
CREATE INDEX credentials_collection ON credentials (collection_id);
