-- One row identifying this installation: names exports and anchors the
-- audit log's hash chain.
CREATE TABLE instance (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    singleton boolean NOT NULL DEFAULT true UNIQUE CHECK (singleton),
    created_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO instance DEFAULT VALUES;
