/**
 * The extension's calls to remotehub (crates/server/src/api/extension.rs):
 * with its bearer token and its version, never with a cookie. An answer
 * `unauthenticated` means the session ended (idle, signed out, ended on
 * *My account*); the extension then forgets it.
 */
import { api, type ApiResult } from '$lib/api/client';
import type { StoredVault } from '$lib/vault/vault';
import { type Connection, connection, forget, touch } from './store';

/** A shared login as the server lists it, without its secrets. */
export interface SharedLogin {
	id: string;
	name: string;
	username: string;
	url: string;
	icon: number;
	has_totp: boolean;
}

export interface Code {
	code: string;
	remaining: number;
	period: number;
}

const version = () => chrome.runtime.getManifest().version;

/** `fetch` against remotehub for this connection, as `api()` takes it. */
function fetcher(server: string, token?: string): typeof fetch {
	return (input, init) =>
		fetch(new URL(String(input), server), {
			...init,
			credentials: 'omit',
			headers: {
				...(init?.headers as Record<string, string>),
				'x-remotehub-extension': version(),
				...(token ? { authorization: `Bearer ${token}` } : {})
			}
		});
}

const NOT_CONNECTED = { ok: false, status: 401, code: 'unauthenticated', params: {} } as const;

async function call<T>(
	method: 'GET' | 'POST' | 'DELETE',
	path: string,
	body?: unknown
): Promise<ApiResult<T>> {
	const current = await connection();
	if (!current) return NOT_CONNECTED;
	const result = await api<T>(method, path, body, fetcher(current.server, current.token));
	if (!result.ok && result.code === 'unauthenticated') await forget();
	else await touch();
	return result;
}

export const loadLogins = () => call<SharedLogin[]>('GET', '/api/extension/entries');

/** User name and password to fill into the page at `origin`; audited. */
export const fillValues = (id: string, origin: string) =>
	call<{ username: string; password: string }>('POST', `/api/extension/entries/${id}/fill`, {
		origin
	});

export const copyPassword = (id: string) =>
	call<{ password: string }>('POST', `/api/extension/entries/${id}/copy`);

export const oneTimeCode = (id: string, purpose: 'fill' | 'copy', origin?: string) =>
	call<Code>('POST', `/api/extension/entries/${id}/code`, { purpose, origin });

export const loadPersonal = () => call<StoredVault>('GET', '/api/extension/personal');

export async function signOut(): Promise<void> {
	await call('DELETE', '/api/extension/session');
	await forget();
}

/** Trades the connect page's code and the PKCE verifier for a session. */
export const trade = (server: string, code: string, verifier: string) =>
	api<{ token: string; username: string; display_name: string; idle_seconds: number }>(
		'POST',
		'/api/extension/token',
		{ code, verifier },
		fetcher(server)
	);

export type { Connection };
