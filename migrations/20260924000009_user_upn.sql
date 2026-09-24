-- The user principal name (alice@example.com) from the directory: RDP with
-- the own account signs in with it, since NTLM needs the domain.
ALTER TABLE users ADD COLUMN upn text;
