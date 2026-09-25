-- Roles for remotehub itself (#106), given to users and groups:
-- administrators manage everything, auditors read the audit log, security
-- officers approve vault recoveries (#95). REMOTEHUB_ADMIN_GROUPS and
-- REMOTEHUB_ADMIN_ACCOUNTS stay as the administrators from the start.
CREATE TABLE role_assignments (
    role text NOT NULL CHECK (role IN ('administrator', 'auditor', 'security_officer')),
    -- Named like the principal of a grant (crate::principal).
    principal_sid text NOT NULL,
    principal_kind text NOT NULL CHECK (principal_kind IN ('user', 'group')),
    -- Name at the time of assigning, for display; the SID decides.
    principal_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users (id),
    PRIMARY KEY (role, principal_sid)
);

CREATE INDEX role_assignments_principal ON role_assignments (principal_sid);
