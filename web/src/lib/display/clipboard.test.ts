import { describe, expect, it } from 'vitest';
import Guacamole from './guacamole/guacamole-common.js';
import { clipboardSync, isPasteKey, mayReadClipboard, type SystemClipboard } from './clipboard';

/** A browser clipboard that refuses writes while `writable` is false. */
function browserClipboard(initial = '') {
	const state = { text: initial, writable: true };
	const system: SystemClipboard = {
		readText: async () => state.text,
		writeText: async (text) => {
			if (!state.writable) throw new DOMException('not allowed', 'NotAllowedError');
			state.text = text;
		}
	};
	return { state, system };
}

function setup(initial = '') {
	const sent: string[] = [];
	const { state, system } = browserClipboard(initial);
	const sync = clipboardSync((text) => sent.push(text), system);
	return { sent, state, sync };
}

function modifiers(set: Partial<Guacamole.Keyboard.ModifierState>) {
	return Object.assign(new Guacamole.Keyboard.ModifierState(), set);
}

describe('clipboard sync', () => {
	it("does not send the session's own text back to it", async () => {
		const { sent, state, sync } = setup();
		sync.fromSession('from the session');
		await sync.flush();
		expect(state.text).toBe('from the session');
		await sync.pull();
		expect(sent).toEqual([]);
	});

	it('sends what was copied in the browser once', async () => {
		const { sent, state, sync } = setup('copied outside');
		await sync.pull();
		await sync.pull();
		state.text = 'copied again';
		await sync.pull();
		expect(sent).toEqual(['copied outside', 'copied again']);
	});

	it("keeps the session's text until the browser lets the page write", async () => {
		const { sent, state, sync } = setup('older');
		state.writable = false;
		sync.fromSession('newer');
		await sync.flush();
		expect(state.text).toBe('older');
		// The older text on the browser's clipboard must not replace the
		// session's newer one.
		await sync.pull();
		expect(sent).toEqual([]);
		state.writable = true;
		await sync.flush();
		expect(state.text).toBe('newer');
	});

	it("lets a paste in the browser win over the session's waiting text", async () => {
		const { sent, state, sync } = setup();
		state.writable = false;
		sync.fromSession('from the session');
		sync.fromBrowser('pasted');
		state.writable = true;
		await sync.flush();
		expect(sent).toEqual(['pasted']);
		expect(state.text).toBe('');
	});

	it('works without a browser clipboard', async () => {
		const sent: string[] = [];
		const sync = clipboardSync((text) => sent.push(text), null);
		sync.fromSession('x');
		await sync.flush();
		await sync.pull();
		sync.fromBrowser('pasted');
		expect(sent).toEqual(['pasted']);
	});

	it('reads the clipboard only where the browser remembers the permission', async () => {
		const nav = (query: () => Promise<{ state: string }>) =>
			({ clipboard: { readText: async () => '' }, permissions: { query } }) as unknown as Navigator;
		expect(await mayReadClipboard(nav(async () => ({ state: 'granted' })))).toBe(true);
		expect(await mayReadClipboard(nav(async () => ({ state: 'prompt' })))).toBe(true);
		expect(await mayReadClipboard(nav(async () => ({ state: 'denied' })))).toBe(false);
		// Firefox: no such permission.
		const unknown = nav(() => Promise.reject(new TypeError('clipboard-read')));
		expect(await mayReadClipboard(unknown)).toBe(false);
		expect(await mayReadClipboard({} as Navigator)).toBe(false);
	});
});

describe('paste keys', () => {
	it('knows the browser shortcuts for paste', () => {
		expect(isPasteKey(0x76, modifiers({ ctrl: true }))).toBe(true);
		expect(isPasteKey(0x56, modifiers({ ctrl: true, shift: true }))).toBe(true);
		expect(isPasteKey(0x76, modifiers({ meta: true }))).toBe(true);
		expect(isPasteKey(0xff63, modifiers({ shift: true }))).toBe(true);
	});

	it('leaves other keys to the session', () => {
		expect(isPasteKey(0x76, modifiers({}))).toBe(false);
		// AltGr+V arrives as Ctrl+Alt+V.
		expect(isPasteKey(0x76, modifiers({ ctrl: true, alt: true }))).toBe(false);
		expect(isPasteKey(0x63, modifiers({ ctrl: true }))).toBe(false);
		expect(isPasteKey(0xff63, modifiers({}))).toBe(false);
		// Ctrl+Insert copies.
		expect(isPasteKey(0xff63, modifiers({ ctrl: true, shift: true }))).toBe(false);
	});
});
