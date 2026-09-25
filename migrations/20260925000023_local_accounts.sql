-- Local accounts in Ory Kratos (#103, ADR 0010). Break-glass accounts get a
-- kind of their own; 'local' now means an account in Kratos, identified by
-- its identity ID like a directory user by its SID.
DROP INDEX users_local_username;
ALTER TABLE users DROP CONSTRAINT users_kind_check;
UPDATE users SET kind = 'break_glass' WHERE kind = 'local';
ALTER TABLE users ADD CONSTRAINT users_kind_check
    CHECK (kind IN ('directory', 'break_glass', 'local', 'deleted'));
CREATE UNIQUE INDEX users_break_glass_username ON users (lower(username))
    WHERE kind = 'break_glass';

ALTER TABLE users ADD COLUMN identity_id uuid UNIQUE;
ALTER TABLE users ADD CONSTRAINT users_local_identity
    CHECK (kind <> 'local' OR identity_id IS NOT NULL);
