-- The names of directory groups (#178). Sign-in reads only their SIDs; the
-- Access page names them. Updated after each sign-in of a member.

CREATE TABLE directory_groups (
    sid text PRIMARY KEY,
    name text NOT NULL,
    seen_at timestamptz NOT NULL DEFAULT now()
);
