-- Folders name a site connector for the devices below them (#176).

-- None: the folder passes on its parent's.
ALTER TABLE folders ADD COLUMN connector_id uuid REFERENCES connectors (id) ON DELETE RESTRICT;
CREATE INDEX folders_connector ON folders (connector_id);

-- A device takes its folder's connector (`inherit`), none (`direct`), or
-- its own (`connector`, then in connector_id). Devices that named a
-- connector keep it; the others take their folder's, which is none yet.
ALTER TABLE devices ADD COLUMN connector_mode text NOT NULL DEFAULT 'inherit'
    CHECK (connector_mode IN ('inherit', 'direct', 'connector'));
UPDATE devices SET connector_mode = 'connector' WHERE connector_id IS NOT NULL;
ALTER TABLE devices ADD CONSTRAINT devices_connector_mode
    CHECK ((connector_mode = 'connector') = (connector_id IS NOT NULL));

-- The connector a folder passes on: its own, or the nearest one above it.
CREATE FUNCTION folder_connector(folder uuid) RETURNS uuid
LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT id, parent_id, connector_id, 0 AS depth FROM folders WHERE id = folder
        UNION ALL
        SELECT f.id, f.parent_id, f.connector_id, up.depth + 1
        FROM folders f JOIN up ON f.id = up.parent_id
        WHERE up.connector_id IS NULL
    )
    SELECT connector_id FROM up WHERE connector_id IS NOT NULL ORDER BY depth LIMIT 1
$$;

-- The connector a device is reached through; none: directly.
CREATE FUNCTION device_connector(mode text, own uuid, folder uuid) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT CASE mode
        WHEN 'connector' THEN own
        WHEN 'direct' THEN NULL
        ELSE folder_connector(folder)
    END
$$;
