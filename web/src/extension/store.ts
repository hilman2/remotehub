/**
 * What the extension keeps, and where (#201, ADR 0017).
 *
 * - `chrome.storage.local`, on disk: remotehub's address as the user typed
 *   it, and the language of their remotehub. Neither is a secret.
 * - `chrome.storage.managed`: the address an administrator set by policy
 *   (`serverUrl`, managed-schema.json); it wins over the typed one.
 * - `chrome.storage.session`, in memory only: the token and the unlocked
 *   personal vault's key. They are gone when the browser closes, and
 *   content scripts cannot read this storage (Chromium's default for it).
 */
import { fromBase64, toBase64 } from '$lib/vault/crypto';

export interface Connection {
	/** remotehub's origin, e.g. `https://remotehub.example.com`. */
	server: string;
	token: string;
	username: string;
	displayName: string;
	/** The session's idle time on the server; the vault locks after as long. */
	idleSeconds: number;
}

/**
 * The origin of a remotehub address as people type it: `https://` added
 * when missing; `http` only for this machine, as in development. Null for
 * anything else.
 */
export function normaliseServer(text: string): string | null {
	const trimmed = text.trim();
	if (!trimmed) return null;
	try {
		const url = new URL(trimmed.includes('://') ? trimmed : `https://${trimmed}`);
		const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
		if (url.protocol === 'https:' || (url.protocol === 'http:' && local)) return url.origin;
		return null;
	} catch {
		return null;
	}
}

/** The host permission remotehub's address needs; Chromium's patterns take no port. */
export const serverPattern = (server: string) => {
	const url = new URL(server);
	return `${url.protocol}//${url.hostname}/*`;
};

export async function serverAddress(): Promise<{ url: string | null; managed: boolean }> {
	try {
		const { serverUrl } = await chrome.storage.managed.get('serverUrl');
		const managed = typeof serverUrl === 'string' ? normaliseServer(serverUrl) : null;
		if (managed) return { url: managed, managed: true };
	} catch {
		// No policy for this extension: the managed storage is empty or absent.
	}
	const { server } = await chrome.storage.local.get('server');
	return { url: typeof server === 'string' ? server : null, managed: false };
}

export const saveServerAddress = (server: string) => chrome.storage.local.set({ server });

export async function connection(): Promise<Connection | null> {
	const { connection } = await chrome.storage.session.get('connection');
	return (connection as Connection | undefined) ?? null;
}

export async function saveConnection(value: Connection): Promise<void> {
	await chrome.storage.session.set({ connection: value, usedAt: Date.now() });
}

/** Signed out: the token and the personal vault's key are gone. */
export const forget = () => chrome.storage.session.remove(['connection', 'vaultKey', 'usedAt']);

/** Notes a use, for locking the personal vault after the idle time. */
export const touch = () => chrome.storage.session.set({ usedAt: Date.now() });

/**
 * The key of the unlocked personal vault, or null. After the idle time
 * without use, the vault locks: the key is dropped here.
 */
export async function vaultKey(): Promise<CryptoKey | null> {
	const { vaultKey, usedAt, connection } = await chrome.storage.session.get([
		'vaultKey',
		'usedAt',
		'connection'
	]);
	if (typeof vaultKey !== 'string' || !connection) return null;
	const idle = (connection as Connection).idleSeconds * 1000;
	if (typeof usedAt !== 'number' || Date.now() - usedAt > idle) {
		await lockVault();
		return null;
	}
	return crypto.subtle.importKey('raw', fromBase64(vaultKey), { name: 'AES-GCM' }, false, [
		'decrypt'
	]);
}

export async function saveVaultKey(key: CryptoKey): Promise<void> {
	const raw = new Uint8Array(await crypto.subtle.exportKey('raw', key));
	await chrome.storage.session.set({ vaultKey: toBase64(raw), usedAt: Date.now() });
}

export const lockVault = () => chrome.storage.session.remove('vaultKey');

/** The language of the user's remotehub, from connecting; null before. */
export async function savedLocale(): Promise<string | null> {
	const { locale } = await chrome.storage.local.get('locale');
	return typeof locale === 'string' ? locale : null;
}

export const saveLocale = (locale: string) => chrome.storage.local.set({ locale });
