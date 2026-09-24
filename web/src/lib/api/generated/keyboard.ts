// Generated from crates/gateway/src/guacamole.rs — do not edit.
// Regenerate: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated

/** guacd's keyboard layouts for RDP sessions (`server-layout`). */
export const KEYBOARD_LAYOUTS = [
	'cs-cz-qwertz',
	'da-dk-qwerty',
	'de-ch-qwertz',
	'de-de-qwertz',
	'en-gb-qwerty',
	'en-us-qwerty',
	'es-es-qwerty',
	'es-latam-qwerty',
	'fr-be-azerty',
	'fr-ca-qwerty',
	'fr-ch-qwertz',
	'fr-fr-azerty',
	'hu-hu-qwertz',
	'it-it-qwerty',
	'ja-jp-qwerty',
	'no-no-qwerty',
	'pl-pl-qwerty',
	'pt-br-qwerty',
	'pt-pt-qwerty',
	'ro-ro-qwerty',
	'sv-se-qwerty',
	'tr-tr-qwerty',
	'failsafe'
] as const;

export type KeyboardLayout = (typeof KEYBOARD_LAYOUTS)[number];
