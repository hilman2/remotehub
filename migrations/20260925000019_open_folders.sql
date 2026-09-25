-- Which folders a user has open in the device tree (#83). A folder without a
-- row is closed: the tree starts with everything closed.
CREATE TABLE open_folders (
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    folder_id uuid NOT NULL REFERENCES folders (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, folder_id)
);
