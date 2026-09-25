-- When the directory was last asked about a session's user (#108): their
-- groups and whether they may still sign in.
ALTER TABLE sessions ADD COLUMN checked_at timestamptz NOT NULL DEFAULT now();
