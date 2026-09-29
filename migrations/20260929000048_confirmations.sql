-- Confirmations with the second factor (#241, #242, #243). A browser's
-- session locks after the idle time instead of ending, and a confirmation
-- unlocks it; showing a secret or connecting to a device that asks for it
-- takes a confirmation of the last minute.
ALTER TABLE sessions
    ADD COLUMN confirmed_at timestamptz,
    -- Wrong confirmations in a row; too many end the session.
    ADD COLUMN confirm_failures integer NOT NULL DEFAULT 0;

ALTER TABLE webauthn_challenges DROP CONSTRAINT webauthn_challenges_purpose_check;
ALTER TABLE webauthn_challenges
    ADD CONSTRAINT webauthn_challenges_purpose_check
    CHECK (purpose IN ('register', 'sign_in', 'confirm'));

-- Local accounts keep their keys and authenticator app in Kratos, which
-- remotehub reads for a confirmation. What Kratos does not know of these
-- confirmations: the last code step used and each key's counter, so that
-- neither answer counts twice.
CREATE TABLE confirmation_codes (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    last_step bigint NOT NULL
);

CREATE TABLE confirmation_keys (
    credential_id bytea PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    sign_count bigint NOT NULL
);

-- Devices that ask for the second factor before every connection (#243).
ALTER TABLE devices ADD COLUMN requires_confirmation boolean NOT NULL DEFAULT false;
