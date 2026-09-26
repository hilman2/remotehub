-- remotehub asks the customer for access through a site connector (#181,
-- ADR 0013). The connector fetches pending requests with its state report
-- and returns the customer's answer the same way.

CREATE TABLE connector_requests (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    connector_id uuid NOT NULL REFERENCES connectors (id) ON DELETE CASCADE,
    -- What the user asked for: a device, or a folder's devices behind the
    -- connector. The targets are fixed when asking, as the customer saw them.
    folder_id uuid REFERENCES folders (id) ON DELETE SET NULL,
    device_id uuid REFERENCES devices (id) ON DELETE SET NULL,
    object_name text NOT NULL,
    -- [{ "name", "host", "port" }], what an approval opens.
    targets jsonb NOT NULL,
    requester_id uuid NOT NULL REFERENCES users (id),
    requester_name text NOT NULL,
    minutes integer NOT NULL CHECK (minutes BETWEEN 15 AND 1440),
    reason text NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    status text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'refused', 'cancelled')),
    -- The connector user who answered, as the connector reports them.
    answered_by text,
    answered_at timestamptz,
    -- The end of an approval.
    until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX connector_requests_pending ON connector_requests (connector_id, created_at)
    WHERE status = 'pending';
CREATE INDEX connector_requests_requester ON connector_requests (requester_id, created_at);
