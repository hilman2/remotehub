-- Search words on devices (#215): one free text that everyone who sees the
-- device may change, and everyone's search finds it by. Who changed it last,
-- by the name they had then, and when.
ALTER TABLE devices
    ADD COLUMN keywords text NOT NULL DEFAULT '' CHECK (length(keywords) <= 500),
    ADD COLUMN keywords_changed_by text,
    ADD COLUMN keywords_changed_at timestamptz;
