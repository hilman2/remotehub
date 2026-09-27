-- The password generator's settings (#194): the organisation's default,
-- set by administrators (user_id NULL), and a default of their own for
-- each user who keeps one. Without a row, the built-in default applies.
CREATE TABLE generator_settings (
    user_id uuid UNIQUE NULLS NOT DISTINCT REFERENCES users (id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('password', 'passphrase')),
    length integer NOT NULL CHECK (length BETWEEN 8 AND 128),
    lower boolean NOT NULL,
    upper boolean NOT NULL,
    digits boolean NOT NULL,
    symbols boolean NOT NULL,
    CHECK (lower OR upper OR digits OR symbols),
    look_alikes boolean NOT NULL,
    words integer NOT NULL CHECK (words BETWEEN 3 AND 20),
    separator text NOT NULL CHECK (char_length(separator) <= 3),
    updated_at timestamptz NOT NULL DEFAULT now()
);
