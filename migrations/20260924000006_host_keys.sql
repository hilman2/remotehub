-- Host keys pinned at the first connection (trust on first use, ADR 0003).
-- A different key later aborts the connection until someone with edit on
-- the device resets the pin (audited).
ALTER TABLE devices
    ADD COLUMN host_key text,
    ADD COLUMN host_key_pinned_at timestamptz;
