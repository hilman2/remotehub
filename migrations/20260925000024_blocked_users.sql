-- Administrators block a user in remotehub (#104): no sign-in and no
-- session, whatever the directory or Kratos says.
ALTER TABLE users ADD COLUMN blocked_at timestamptz;
