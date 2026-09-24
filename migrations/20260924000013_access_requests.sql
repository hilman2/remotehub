-- Just-in-time access (#20): a user asks for a role on an object for a
-- while, someone else who manages the object approves, and the approval
-- becomes a grant that ends by itself.

-- A time-limited grant counts until expires_at; authorize() ignores it
-- afterwards. The row stays as the record of what was granted.
ALTER TABLE grants
    ADD COLUMN expires_at timestamptz,
    ADD COLUMN request_id uuid;

-- One permanent grant per principal and object, as before; time-limited
-- grants may exist beside it.
ALTER TABLE grants DROP CONSTRAINT grants_folder_id_device_id_credential_id_principal_sid_key;
CREATE UNIQUE INDEX grants_permanent ON grants (folder_id, device_id, credential_id, principal_sid)
    NULLS NOT DISTINCT WHERE expires_at IS NULL;

CREATE TABLE access_requests (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id uuid REFERENCES folders (id) ON DELETE CASCADE,
    device_id uuid REFERENCES devices (id) ON DELETE CASCADE,
    credential_id uuid REFERENCES credentials (id) ON DELETE CASCADE,
    requester_id uuid NOT NULL REFERENCES users (id),
    -- The requester's own SID: the grant goes to it, and nobody approves
    -- their own request.
    requester_sid text NOT NULL,
    requester_name text NOT NULL,
    role text NOT NULL CHECK (role IN ('connect', 'reveal')),
    minutes integer NOT NULL CHECK (minutes BETWEEN 15 AND 1440),
    reason text NOT NULL CHECK (length(trim(reason)) BETWEEN 1 AND 500),
    status text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'denied', 'cancelled')),
    decided_by uuid REFERENCES users (id),
    decider_name text,
    decided_at timestamptz,
    expires_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (num_nonnulls(folder_id, device_id, credential_id) = 1)
);

CREATE INDEX access_requests_pending ON access_requests (status) WHERE status = 'pending';

ALTER TABLE grants ADD CONSTRAINT grants_request_fkey
    FOREIGN KEY (request_id) REFERENCES access_requests (id) ON DELETE SET NULL;
