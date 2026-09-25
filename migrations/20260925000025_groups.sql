-- Groups of remotehub's own (#105): for installations without a directory,
-- and for teams the directory does not know. A group is named in grants and
-- elsewhere as 'group:<id>'.
CREATE TABLE groups (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users (id)
);

CREATE UNIQUE INDEX groups_name ON groups (lower(name));

-- Members are users (a directory SID or 'local:<Kratos identity>') and
-- directory groups by SID; groups of remotehub's own do not nest.
CREATE TABLE group_members (
    group_id uuid NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    principal_sid text NOT NULL,
    principal_kind text NOT NULL CHECK (principal_kind IN ('user', 'group')),
    -- Name at the time of adding, for display; the SID decides.
    principal_name text NOT NULL,
    added_at timestamptz NOT NULL DEFAULT now(),
    added_by uuid REFERENCES users (id),
    PRIMARY KEY (group_id, principal_sid),
    CHECK (principal_sid NOT LIKE 'group:%')
);

-- The session lookup finds a user's groups by their principals.
CREATE INDEX group_members_principal ON group_members (principal_sid);
