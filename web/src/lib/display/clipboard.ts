/**
 * Text clipboard between the browser and an RDP or VNC session. guacd keeps
 * the session's clipboard in step with the target and exchanges it with the
 * browser as Guacamole `clipboard` streams.
 */
import Guacamole from './guacamole/guacamole-common.js';

/** The part of `navigator.clipboard` the sync uses. */
export interface SystemClipboard {
	readText(): Promise<string>;
	writeText(text: string): Promise<void>;
}

export interface ClipboardSync {
	/** The session's clipboard changed to `text`. */
	fromSession(text: string): void;
	/** The browser's clipboard holds `text`, e.g. from a paste. */
	fromBrowser(text: string): void;
	/**
	 * Writes the session's text to the browser's clipboard, if some is still
	 * waiting. Firefox and Safari write only during a user's action, so call
	 * it again on the next key or click in the session.
	 */
	flush(): Promise<void>;
	/** Reads the browser's clipboard and passes it on, where the browser allows it. */
	pull(): Promise<void>;
}

/**
 * Keeps the browser's clipboard and the session's in step. `send` passes
 * text to the session; `system` is null where the browser has no clipboard
 * API (plain http other than localhost).
 */
export function clipboardSync(
	send: (text: string) => void,
	system: SystemClipboard | null
): ClipboardSync {
	// The text both sides are known to hold. What the session sent comes back
	// from the browser's clipboard on the next pull and must not be sent again.
	let shared: string | null = null;
	// Text from the session that the browser's clipboard did not take yet.
	let pending: string | null = null;

	const sync: ClipboardSync = {
		fromSession(text) {
			shared = text;
			pending = text;
			void sync.flush();
		},
		fromBrowser(text) {
			if (text === '' || text === shared) return;
			shared = text;
			// Copied in the browser after the session's copy: the newer one wins.
			pending = null;
			send(text);
		},
		async flush() {
			const text = pending;
			if (text === null || !system) return;
			try {
				await system.writeText(text);
				if (pending === text) pending = null;
			} catch {
				// Not allowed right now; the next flush tries again.
			}
		},
		async pull() {
			if (!system) return;
			// While the session's text waits, the browser's clipboard holds
			// something older, which must not overwrite the session's.
			await sync.flush();
			if (pending !== null) return;
			try {
				sync.fromBrowser(await system.readText());
			} catch {
				// No permission, or the page is not focused.
			}
		}
	};
	return sync;
}

/**
 * Whether the browser lets the page read the clipboard without a prompt on
 * every read. Chromium asks once and remembers; Firefox and Safari know no
 * `clipboard-read` permission and ask on every read, so there the sync waits
 * for a paste instead.
 */
export async function mayReadClipboard(nav: Navigator = navigator): Promise<boolean> {
	if (!nav.clipboard?.readText || !nav.permissions) return false;
	try {
		const status = await nav.permissions.query({ name: 'clipboard-read' as PermissionName });
		return status.state !== 'denied';
	} catch {
		return false;
	}
}

const KEYSYM_V = 0x76;
const KEYSYM_SHIFTED_V = 0x56;
const KEYSYM_INSERT = 0xff63;

/**
 * Whether the key pastes in the browser: Ctrl+V, Cmd+V or Shift+Insert.
 * Ctrl+Alt+V is AltGr+V on many layouts and types a character instead.
 */
export function isPasteKey(keysym: number, modifiers: Guacamole.Keyboard.ModifierState): boolean {
	if (keysym === KEYSYM_V || keysym === KEYSYM_SHIFTED_V) {
		return (modifiers.ctrl && !modifiers.alt) || modifiers.meta;
	}
	return keysym === KEYSYM_INSERT && modifiers.shift && !modifiers.ctrl;
}

/** A clipboard sync on the Guacamole client's clipboard streams. */
export function attachClipboard(
	client: Guacamole.Client,
	system: SystemClipboard | null
): ClipboardSync {
	const sync = clipboardSync((text) => {
		const writer = new Guacamole.StringWriter(client.createClipboardStream('text/plain'));
		writer.sendText(text);
		writer.sendEnd();
	}, system);
	client.onclipboard = (stream, mimetype) => {
		if (!mimetype.startsWith('text/')) {
			stream.sendAck('Unsupported', 0x0100);
			return;
		}
		const reader = new Guacamole.StringReader(stream);
		let text = '';
		reader.ontext = (chunk) => (text += chunk);
		reader.onend = () => sync.fromSession(text);
	};
	return sync;
}
