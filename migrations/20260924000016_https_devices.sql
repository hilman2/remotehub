-- Web interfaces of devices, opened in the browser service (#16, ADR 0007).
-- Their certificate is pinned in certificate_fingerprint, like RDP's.
ALTER TABLE devices DROP CONSTRAINT devices_protocol_check;
ALTER TABLE devices ADD CONSTRAINT devices_protocol_check
    CHECK (protocol IN ('ssh', 'rdp', 'vnc', 'https'));
