-- Audit log (ADR 0004): append-only and hash-chained.
--
-- Every entry stores the hash of its predecessor and its own hash over the
-- predecessor's hash and all of its fields. Changing, deleting or inserting
-- an entry afterwards breaks the chain from that point on; `remotehub
-- verify-audit` finds the first broken entry. Triggers refuse UPDATE,
-- DELETE and TRUNCATE; a database superuser can still bypass them, which is
-- what the hash chain is for.
CREATE TABLE audit_log (
    seq bigint PRIMARY KEY,
    at timestamptz NOT NULL DEFAULT now(),
    -- NULL when nobody is signed in, e.g. a failed sign-in.
    actor_id uuid REFERENCES users (id),
    -- The name as it was at the time (or as typed at a failed sign-in).
    actor_name text NOT NULL,
    action text NOT NULL,
    object_type text,
    object_id uuid,
    details jsonb NOT NULL DEFAULT '{}',
    address text,
    prev_hash bytea NOT NULL,
    hash bytea NOT NULL
);

CREATE INDEX audit_log_at ON audit_log (at);
CREATE INDEX audit_log_actor ON audit_log (actor_id);
CREATE INDEX audit_log_object ON audit_log (object_type, object_id);

-- The hash of an entry. A JSON array is unambiguous whatever the fields
-- contain, and jsonb's text form is deterministic; the time goes in as
-- microseconds so neither DateStyle nor TimeZone can change the result.
CREATE FUNCTION audit_digest(
    prev_hash bytea, seq bigint, at timestamptz, actor_id uuid, actor_name text,
    action text, object_type text, object_id uuid, details jsonb, address text
) RETURNS bytea LANGUAGE sql IMMUTABLE AS $$
    SELECT sha256(prev_hash || convert_to(jsonb_build_array(
        seq, (extract(epoch FROM at) * 1000000)::bigint, actor_id, actor_name,
        action, object_type, object_id, details, address
    )::text, 'UTF8'))
$$;

-- Start of the chain, bound to this installation.
CREATE FUNCTION audit_genesis() RETURNS bytea LANGUAGE sql STABLE AS $$
    SELECT sha256(convert_to('remotehub-audit|' || id::text, 'UTF8')) FROM instance
$$;

-- Numbers and chains every new entry. The advisory lock serialises writers
-- until their transaction ends, so the numbers follow the commit order and
-- have no gaps.
CREATE FUNCTION audit_chain() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    last record;
BEGIN
    PERFORM pg_advisory_xact_lock(7271827);
    SELECT a.seq, a.hash INTO last FROM audit_log a ORDER BY a.seq DESC LIMIT 1;
    NEW.seq := coalesce(last.seq, 0) + 1;
    NEW.at := coalesce(NEW.at, now());
    NEW.prev_hash := coalesce(last.hash, audit_genesis());
    NEW.hash := audit_digest(NEW.prev_hash, NEW.seq, NEW.at, NEW.actor_id, NEW.actor_name,
        NEW.action, NEW.object_type, NEW.object_id, NEW.details, NEW.address);
    RETURN NEW;
END
$$;

CREATE TRIGGER audit_log_chain BEFORE INSERT ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_chain();

CREATE FUNCTION audit_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_log is append-only';
END
$$;

CREATE TRIGGER audit_log_no_change BEFORE UPDATE OR DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_append_only();
CREATE TRIGGER audit_log_no_truncate BEFORE TRUNCATE ON audit_log
    FOR EACH STATEMENT EXECUTE FUNCTION audit_append_only();
