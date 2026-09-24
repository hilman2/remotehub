-- SSH devices can sign in with a short-lived certificate from remotehub's
-- own CA, as the signed-in user (#17).
ALTER TABLE devices DROP CONSTRAINT devices_auth_mode_check;
ALTER TABLE devices ADD CONSTRAINT devices_auth_mode_check
    CHECK (auth_mode IN ('stored', 'ask', 'own', 'certificate'));
