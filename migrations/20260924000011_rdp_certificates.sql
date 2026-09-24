-- RDP certificates pinned on first use, like SSH host keys: the SHA-256
-- fingerprint of the certificate the device presented (AA:BB:…).
ALTER TABLE devices
    ADD COLUMN certificate_fingerprint text,
    ADD COLUMN certificate_pinned_at timestamptz;
