-- "Connect with my own directory account" (ADR 0005): the sign-in password,
-- encrypted with a key that exists only in the user's cookie
-- (__Host-remotehub-login-key) and bound to this session. A copy of this
-- table cannot open it; it disappears with the session.
ALTER TABLE sessions ADD COLUMN login_secret bytea;
