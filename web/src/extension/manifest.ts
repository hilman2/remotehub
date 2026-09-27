/**
 * The extension's manifest (Manifest V3), written by vite.extension.config.ts.
 *
 * The permissions are what filling on request needs, and no more: the
 * extension reaches a page only when the user asks in its tab (`activeTab`:
 * popup, shortcut, context menu), and remotehub's own host only after the
 * user named it at setup (`optional_host_permissions`). It reads no tabs'
 * addresses in the background and runs no script in any page by itself.
 */

/** Names the texts of `_locales`, which the build takes from messages/{locale}.json. */
export const LOCALE_KEYS = ['extension_name', 'extension_description', 'extension_command_fill'];

/**
 * @param version The release's version, as in the workspace's Cargo.toml.
 * @param key The public key (SPKI, base64) whose hash is the extension's ID,
 *     so a build loaded unpacked has the ID of the signed CRX.
 * @param testing For the end-to-end tests only: access to every page from the
 *     start. A browser under Playwright gets no clicks on its toolbar or on
 *     permission prompts, which grant access to a tab or to remotehub.
 */
export function manifest(version: string, key: string, testing = false) {
	return {
		...(testing ? { host_permissions: ['<all_urls>'] } : {}),
		manifest_version: 3,
		name: '__MSG_extension_name__',
		description: '__MSG_extension_description__',
		default_locale: 'en',
		version,
		key,
		// chrome.action.openPopup() for the shortcut, and WebAuthn with
		// remotehub's host as relying party (122).
		minimum_chrome_version: '127',
		icons: { 16: 'icons/16.png', 32: 'icons/32.png', 48: 'icons/48.png', 128: 'icons/128.png' },
		action: {
			default_popup: 'popup.html',
			default_title: '__MSG_extension_name__',
			default_icon: { 16: 'icons/16.png', 32: 'icons/32.png' }
		},
		options_page: 'options.html',
		background: { service_worker: 'background.js', type: 'module' },
		permissions: [
			'activeTab',
			'scripting',
			'storage',
			'identity',
			'contextMenus',
			'clipboardWrite',
			// Clearing a copied secret from the clipboard later (clipboard.ts).
			'alarms',
			'offscreen'
		],
		// Only remotehub's address from these, asked for at setup.
		optional_host_permissions: ['https://*/*', 'http://*/*'],
		commands: {
			fill: {
				suggested_key: { default: 'Ctrl+Shift+L', mac: 'Command+Shift+L' },
				description: '__MSG_extension_command_fill__'
			}
		},
		storage: { managed_schema: 'managed-schema.json' }
	};
}
