/**
 * Confirming with the second factor that it is still you (#241, #242,
 * #243). A locked session shows the lock screen over the page; a secret or
 * a device that asks for it shows a dialog. Both wait for one confirmation:
 * requests refused meanwhile are sent again once it succeeds.
 */
import { api, background } from '$lib/api/client';

/** What the user can confirm with: an app's code, a key's challenge. */
export interface ConfirmStart {
	app: boolean;
	key: { challenge_id: string; options: unknown } | null;
}

/**
 * Asked again while the page waits for a confirmation (#253), so it does
 * not keep the session alive: an open dialog must not stop it from locking.
 */
export const startConfirmation = () =>
	api<ConfirmStart>('POST', '/api/session/confirm/start', undefined, background);

export type ConfirmAnswer =
	{ code: string } | { key: { challenge_id: string; credential: unknown } };

export const sendConfirmation = (answer: ConfirmAnswer) =>
	api('POST', '/api/session/confirm', answer);

/** Whether the lock screen or the dialog is up. */
export const confirming = $state<{ locked: boolean; asked: boolean }>({
	locked: false,
	asked: false
});

let unlocking: ((unlocked: boolean) => void)[] = [];
let asking: ((confirmed: boolean) => void)[] = [];

/** Shows the lock screen; resolves once the session is unlocked or over. */
export function lockScreen(): Promise<boolean> {
	confirming.locked = true;
	return new Promise((resolve) => unlocking.push(resolve));
}

/** Shows the dialog; resolves once confirmed, or false if it was closed. */
export function askConfirmation(): Promise<boolean> {
	// Behind a lock screen, its confirmation holds for both.
	if (!confirming.locked) confirming.asked = true;
	return new Promise((resolve) => asking.push(resolve));
}

/** A confirmation succeeded, or the user gave up (`false`). */
export function confirmed(ok: boolean) {
	const waiting = [...unlocking, ...asking];
	unlocking = [];
	asking = [];
	confirming.locked = false;
	confirming.asked = false;
	for (const resolve of waiting) resolve(ok);
}

/** The dialog was closed without a confirmation; a lock screen stays. */
export function declined() {
	const waiting = asking;
	asking = [];
	confirming.asked = false;
	for (const resolve of waiting) resolve(false);
}

/**
 * Covers the page in time: after `idleSeconds` without keys, clicks or
 * touches here — typing into a connection counts too — it asks the server
 * whether the session is locked, without keeping it alive. Activity here
 * that sends no request keeps the session alive every few minutes, as the
 * server cannot see it. Returns the function that stops watching.
 */
export function watchIdle(idleSeconds: number, onLocked: () => void): () => void {
	let lastInput = Date.now();
	let lastKept = Date.now();
	const input = () => (lastInput = Date.now());
	const events = ['keydown', 'pointerdown', 'wheel', 'touchstart'] as const;
	for (const event of events)
		window.addEventListener(event, input, { capture: true, passive: true });
	const timer = setInterval(async () => {
		if (confirming.locked) return;
		const now = Date.now();
		if (now - lastInput >= idleSeconds * 1000) {
			const me = await api<{ locked: boolean }>('GET', '/api/session', undefined, background);
			if (me.ok && me.data.locked) onLocked();
		} else if (lastInput > lastKept && now - lastKept >= KEEP_ALIVE_MS) {
			lastKept = now;
			await api('GET', '/api/session');
		}
	}, CHECK_MS);
	return () => {
		clearInterval(timer);
		for (const event of events) window.removeEventListener(event, input, { capture: true });
	};
}

const CHECK_MS = 30_000;
const KEEP_ALIVE_MS = 5 * 60_000;
