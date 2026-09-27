/**
 * Connecting the extension to remotehub (#201, ADR 0017): OAuth 2.0's
 * authorization code with PKCE, through the browser's own sign-in window.
 * remotehub's page `/extension/connect` signs the user in as ever, asks them
 * to allow it, and sends a code to the address Chromium reserves for this
 * extension, which `launchWebAuthFlow` catches. The code and the verifier
 * then buy a token of the extension's own.
 */
import { locales } from '$lib/i18n';
import { pkce, randomToken } from './pkce';
import { trade } from './server';
import { saveConnection, saveLocale } from './store';

export type Connected = { ok: true } | { ok: false; code: string };

export async function connectTo(server: string): Promise<Connected> {
	const { verifier, challenge } = await pkce();
	const state = randomToken(24);
	const page = new URL('/extension/connect', server);
	page.search = new URLSearchParams({
		extension_id: chrome.runtime.id,
		challenge,
		state
	}).toString();

	let answer: URL;
	try {
		const back = await chrome.identity.launchWebAuthFlow({ url: page.href, interactive: true });
		answer = new URL(back ?? '');
	} catch {
		// The user closed the window, or remotehub could not be reached.
		return { ok: false, code: 'extension_connect_cancelled' };
	}
	// An answer to another request is none: `state` ties it to this one.
	if (answer.searchParams.get('state') !== state) {
		return { ok: false, code: 'extension_connect_failed' };
	}
	if (answer.searchParams.has('error')) return { ok: false, code: 'extension_connect_cancelled' };

	const traded = await trade(server, answer.searchParams.get('code') ?? '', verifier);
	if (!traded.ok) return { ok: false, code: traded.code };
	await saveConnection({
		server,
		token: traded.data.token,
		username: traded.data.username,
		displayName: traded.data.display_name,
		idleSeconds: traded.data.idle_seconds
	});
	const locale = answer.searchParams.get('locale') ?? '';
	if ((locales as readonly string[]).includes(locale)) await saveLocale(locale);
	return { ok: true };
}
