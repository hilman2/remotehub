/**
 * The browser extension from remotehub's side (#201, ADR 0017): the page
 * that connects it, the extensions of the user on *My account*, and where
 * remotehub serves it (crates/server/src/api/extension.rs, downloads.rs).
 */
import { resolve } from '$app/paths';
import type { ResolvedPathname } from '$app/types';
import { api } from '$lib/api/client';

/** What the extension sends to `/extension/connect`, in the query. */
export interface ConnectRequest {
	/** Chromium's ID of the extension: 32 letters from a to p. */
	extensionId: string;
	/** BASE64URL(SHA-256(verifier)), PKCE's method S256. */
	challenge: string;
	/** Returned as it came, so the extension knows the answer is to its request. */
	state: string;
}

/** The request in a query; null if something is missing or malformed. */
export function connectRequest(query: URLSearchParams): ConnectRequest | null {
	const extensionId = query.get('extension_id') ?? '';
	const challenge = query.get('challenge') ?? '';
	const state = query.get('state') ?? '';
	if (!/^[a-p]{32}$/.test(extensionId)) return null;
	if (!/^[A-Za-z0-9_-]{43}$/.test(challenge)) return null;
	if (!/^[A-Za-z0-9_-]{16,128}$/.test(state)) return null;
	return { extensionId, challenge, state };
}

/**
 * Where the answer goes: the address Chromium reserves for the extension
 * with this ID, which `chrome.identity.launchWebAuthFlow` catches before it
 * reaches the network. Built from the ID alone, so no request can send the
 * code anywhere else.
 */
export function answerUrl(
	request: ConnectRequest,
	answer: { code: string; locale: string } | { error: 'denied' }
): string {
	const url = new URL(`https://${request.extensionId}.chromiumapp.org/`);
	url.searchParams.set('state', request.state);
	if ('code' in answer) {
		url.searchParams.set('code', answer.code);
		url.searchParams.set('locale', answer.locale);
	} else {
		url.searchParams.set('error', answer.error);
	}
	return url.href;
}

/** The connect page with a request's query, as `resolve()` would give it. */
export const connectHref = (query: string) =>
	`${resolve('/extension/connect')}${query}` as ResolvedPathname;

// The connect request waits here while the user signs in: every way of
// signing in ends on the start page, and the layout comes back to it. The
// tab's session storage, so another tab never picks it up.
const PENDING = 'remotehub-extension-connect';
/** A request older than this is dropped; the extension has given up on it. */
const PENDING_MS = 10 * 60 * 1000;

export function rememberConnect(query: string): void {
	try {
		sessionStorage.setItem(PENDING, JSON.stringify({ query, at: Date.now() }));
	} catch {
		// Without storage the user starts over from the extension.
	}
}

/** The query of a request that waited for sign-in, once; null if none. */
export function takePendingConnect(): string | null {
	try {
		const stored = sessionStorage.getItem(PENDING);
		sessionStorage.removeItem(PENDING);
		if (!stored) return null;
		const { query, at } = JSON.parse(stored) as { query: string; at: number };
		return Date.now() - at < PENDING_MS && query.startsWith('?') ? query : null;
	} catch {
		return null;
	}
}

/**
 * The browser and system, for the user to recognise the connection on
 * *My account*: e.g. `Edge · Windows`. Product names, never translated.
 */
export function browserName(
	agent: { brands?: { brand: string }[]; platform?: string } | undefined,
	userAgent: string
): string {
	const known = ['Microsoft Edge', 'Google Chrome', 'Brave', 'Opera', 'Vivaldi', 'Chromium'];
	const brand =
		known.find((name) => agent?.brands?.some((b) => b.brand === name)) ??
		(/Edg\//.test(userAgent)
			? 'Microsoft Edge'
			: /OPR\//.test(userAgent)
				? 'Opera'
				: /Chrome\//.test(userAgent)
					? 'Google Chrome'
					: 'Chromium');
	const platform =
		agent?.platform ||
		(/Windows/.test(userAgent)
			? 'Windows'
			: /Mac OS X/.test(userAgent)
				? 'macOS'
				: /Linux/.test(userAgent)
					? 'Linux'
					: '');
	const short = brand.replace(/^(Microsoft|Google) /, '');
	return platform ? `${short} · ${platform}` : short;
}

export const askCode = (request: ConnectRequest, name: string) =>
	api<{ code: string }>('POST', '/api/extension-codes', {
		extension_id: request.extensionId,
		challenge: request.challenge,
		name
	});

export interface ConnectedExtension {
	id: string;
	name: string;
	/** RFC 3339, UTC */
	created_at: string;
	/** RFC 3339, UTC, to the minute */
	last_seen_at: string;
}

export const loadExtensions = () => api<ConnectedExtension[]>('GET', '/api/account/extensions');
export const endExtension = (id: string) => api('DELETE', `/api/account/extensions/${id}`);

/** Where remotehub serves the extension (crates/server/src/downloads.rs). */
export const EXTENSION_DOWNLOADS = {
	zip: '/downloads/remotehub-extension.zip',
	updates: '/downloads/remotehub-extension.xml',
	sums: '/downloads/SHA256SUMS'
};

/**
 * Whether remotehub serves the extension, and whether also signed for
 * installation by policy. The hash lines name only what is served.
 */
export async function servedExtension(): Promise<{ zip: boolean; crx: boolean }> {
	try {
		const response = await fetch(EXTENSION_DOWNLOADS.sums);
		const sums = response.ok ? await response.text() : '';
		return {
			zip: sums.includes('remotehub-extension.zip'),
			crx: sums.includes('remotehub-extension.crx')
		};
	} catch {
		return { zip: false, crx: false };
	}
}

/** The extension's ID, as the update manifest names it; null without one. */
export async function servedExtensionId(): Promise<string | null> {
	try {
		const response = await fetch(EXTENSION_DOWNLOADS.updates);
		if (!response.ok) return null;
		return /appid='([a-p]{32})'/.exec(await response.text())?.[1] ?? null;
	} catch {
		return null;
	}
}
