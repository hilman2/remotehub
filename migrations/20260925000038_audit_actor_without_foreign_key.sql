-- An audit entry no longer locks its actor's row (#152). The foreign key
-- made every entry ask for FOR KEY SHARE on the user's row while holding the
-- chain's advisory lock, and a sign-in holds that row while it waits for the
-- advisory lock: the two deadlocked. Users are never deleted, so the key
-- guarded nothing; actor_name keeps the name as it was.
ALTER TABLE audit_log DROP CONSTRAINT audit_log_actor_id_fkey;
