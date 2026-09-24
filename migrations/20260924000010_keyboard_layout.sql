-- Keyboard layout of an RDP device's session (guacd's `server-layout`);
-- NULL uses the instance's default (REMOTEHUB_RDP_KEYBOARD_LAYOUT).
ALTER TABLE devices ADD COLUMN keyboard_layout text;
