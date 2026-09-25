/**
 * The personal vault in the browser: unlocking, the entries, the ways to
 * unlock (crypto.ts has the cryptography, crates/server/src/api/personal.rs
 * the storage). The vault key lives only in memory while unlocked.
 */
import { api, lasting } from '$lib/api/client';
import type { Pick } from '$lib/search/rank';
import {
	PBKDF2_ITERATIONS,
	fromBase64,
	newRecoveryKey,
	newVaultKey,
	open,
	openFile,
	parseRecoveryKey,
	passphraseKey,
	randomBytes,
	seal,
	sealFile,
	secretKey,
	toBase64,
	unwrapKey,
	wrapKey
} from './crypto';

export type UnlockKind = 'passkey' | 'passphrase' | 'recovery';

export interface Unlock {
	id: string;
	kind: UnlockKind;
	params: Record<string, unknown>;
	wrapped_key: string;
	label: string;
}

export interface StoredVault {
	scheme: string;
	unlocks: Unlock[];
	entries: { id: string; nonce: string; ciphertext: string }[];
	/** What the owner picked after searching, sealed; null before the first pick. */
	search: { nonce: string; ciphertext: string } | null;
}

/**
 * The associated data of the sealed picks: not an entry's UUID, so neither
 * can be passed off as the other.
 */
const SEARCH_ID = 'search';

export interface EntryContent {
	/** A folder of the personal vault (#98); an entry otherwise. */
	kind?: 'folder';
	/** The folder it lies in; the top without one. */
	parent?: string | null;
	title: string;
	username: string;
	password: string;
	url: string;
	notes: string;
	/** One of KeePass' standard icons (icons.ts). */
	icon?: number;
	/** Custom fields; protected ones are shown only on request. */
	fields?: { name: string; value: string; protected: boolean }[];
	/** Files, sealed apart (#100); here only what finds and names them. */
	attachments?: FileRef[];
	/** Earlier states of the entry, newest first (#100). */
	history?: EarlierContent[];
}

export interface FileRef {
	id: string;
	name: string;
	size: number;
	type: string;
}

/** An entry as it was, with when it stopped being so. */
export type EarlierContent = Omit<EntryContent, 'history' | 'attachments' | 'kind'> & {
	at: string;
};

/** How many earlier states an entry keeps. */
const HISTORY = 10;
/** Sealed entries hold at most 64 KiB; the history makes room for the rest. */
const ENTRY_BUDGET = 60_000;

/**
 * `next` with `before` added to its history, as far as the entry's size
 * allows; unchanged content adds nothing.
 */
export function withHistory(before: EntryContent, next: EntryContent): EntryContent {
	/** What a version of the entry is, without history and files. */
	const state = (content: EntryContent) => ({
		title: content.title,
		username: content.username,
		password: content.password,
		url: content.url,
		notes: content.notes,
		fields: content.fields,
		icon: content.icon,
		parent: content.parent
	});
	if (JSON.stringify(state(before)) === JSON.stringify(state(next))) {
		return { ...next, history: before.history };
	}
	const earlier: EarlierContent = { ...state(before), at: new Date().toISOString() };
	const history = [earlier, ...(before.history ?? [])].slice(0, HISTORY);
	while (history.length > 0 && JSON.stringify({ ...next, history }).length > ENTRY_BUDGET) {
		history.pop();
	}
	return { ...next, history };
}

/** The largest file of the personal vault, in bytes. */
export const MAX_FILE = 5 * 1024 * 1024;

/** Seals a file and stores it; its reference goes into the entry. */
export async function saveFile(key: CryptoKey, file: File): Promise<FileRef | null> {
	const id = globalThis.crypto.randomUUID();
	const bytes = new Uint8Array(await file.arrayBuffer());
	const { nonce, ciphertext } = await sealFile(key, id, bytes);
	const result = await api('PUT', `/api/personal/attachments/${id}`, {
		nonce: toBase64(nonce),
		ciphertext: toBase64(ciphertext)
	});
	if (!result.ok) return null;
	return { id, name: file.name, size: file.size, type: file.type || 'application/octet-stream' };
}

/** The file's bytes, opened in the browser. */
export async function readFile(key: CryptoKey, ref: FileRef): Promise<Blob | null> {
	const result = await api<{ nonce: string; ciphertext: string }>(
		'GET',
		`/api/personal/attachments/${ref.id}`
	);
	if (!result.ok) return null;
	const bytes = await openFile(
		key,
		ref.id,
		fromBase64(result.data.nonce),
		fromBase64(result.data.ciphertext)
	);
	return new Blob([bytes], { type: ref.type });
}

export const deleteFile = (id: string) => api('DELETE', `/api/personal/attachments/${id}`);

/** An entry as read; `content` is null if it would not open with this key. */
export interface Entry {
	id: string;
	content: EntryContent | null;
}

export const loadVault = () => api<StoredVault>('GET', '/api/personal/vault');
export const resetVault = () => api('DELETE', '/api/personal/vault');
export const removeUnlock = (id: string) => api('DELETE', `/api/personal/unlocks/${id}`);
export const deleteEntry = (id: string) => api('DELETE', `/api/personal/entries/${id}`);

function addUnlock(
	kind: UnlockKind,
	params: Record<string, unknown>,
	wrapped: Uint8Array,
	label = ''
) {
	return api<{ id: string }>('POST', '/api/personal/unlocks', {
		kind,
		params,
		wrapped_key: toBase64(wrapped),
		label
	});
}

/** Sets up a new vault. Returns its key and the recovery key, which is shown once. */
export async function setUp(passphrase: string) {
	const key = await newVaultKey();
	const recovery = newRecoveryKey();
	const [withPassphrase, withRecovery] = await Promise.all([
		savePassphrase(key, passphrase),
		saveRecovery(key, recovery.bytes)
	]);
	if (!withPassphrase.ok) return withPassphrase;
	if (!withRecovery.ok) return withRecovery;
	return { ok: true as const, key, recovery: recovery.text };
}

export async function savePassphrase(key: CryptoKey, passphrase: string) {
	const salt = randomBytes(16);
	const wrapped = await wrapKey(key, await passphraseKey(passphrase, salt));
	return addUnlock('passphrase', { salt: toBase64(salt), iterations: PBKDF2_ITERATIONS }, wrapped);
}

function saveRecovery(key: CryptoKey, bytes: Uint8Array<ArrayBuffer>) {
	return secretKey(bytes, 'recovery').then(async (wrapping) =>
		addUnlock('recovery', {}, await wrapKey(key, wrapping))
	);
}

/** A new recovery key replaces the old one; returns its text. */
export async function renewRecovery(key: CryptoKey) {
	const recovery = newRecoveryKey();
	const saved = await saveRecovery(key, recovery.bytes);
	return saved.ok ? { ok: true as const, recovery: recovery.text } : saved;
}

function find(vault: StoredVault, kind: UnlockKind) {
	return vault.unlocks.find((unlock) => unlock.kind === kind);
}

/** The vault key, or null if the passphrase is wrong. */
export async function unlockWithPassphrase(vault: StoredVault, passphrase: string) {
	const unlock = find(vault, 'passphrase');
	if (!unlock) return null;
	const salt = fromBase64(String(unlock.params.salt));
	const iterations = Number(unlock.params.iterations);
	try {
		return await unwrapKey(
			fromBase64(unlock.wrapped_key),
			await passphraseKey(passphrase, salt, iterations)
		);
	} catch {
		return null;
	}
}

/** The vault key, or null if the recovery key is malformed or wrong. */
export async function unlockWithRecovery(vault: StoredVault, text: string) {
	const unlock = find(vault, 'recovery');
	const bytes = parseRecoveryKey(text);
	if (!unlock || !bytes) return null;
	try {
		return await unwrapKey(fromBase64(unlock.wrapped_key), await secretKey(bytes, 'recovery'));
	} catch {
		return null;
	}
}

// WebAuthn's PRF extension: a secret per credential and salt, which the
// authenticator computes only after user verification.
type PrfResults = { prf?: { enabled?: boolean; results?: { first?: ArrayBuffer } } };

function base64url(bytes: Uint8Array): string {
	return toBase64(bytes).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
	const padded = text.replaceAll('-', '+').replaceAll('_', '/');
	return fromBase64(padded + '='.repeat((4 - (padded.length % 4)) % 4));
}

/** Whether the browser offers passkeys at all; PRF support shows only when adding one. */
export function passkeysAvailable(): boolean {
	return typeof window !== 'undefined' && 'PublicKeyCredential' in window;
}

/**
 * Registers a passkey and wraps the vault key with its PRF output. Returns
 * `prf_unsupported` when the authenticator cannot derive secrets.
 */
export async function addPasskey(key: CryptoKey, username: string, label: string) {
	const credential = (await navigator.credentials.create({
		publicKey: {
			rp: { name: 'remotehub' },
			user: { id: randomBytes(16), name: username, displayName: username },
			challenge: randomBytes(32),
			pubKeyCredParams: [
				{ type: 'public-key', alg: -7 },
				{ type: 'public-key', alg: -257 }
			],
			authenticatorSelection: { residentKey: 'preferred', userVerification: 'required' },
			extensions: { prf: {} } as AuthenticationExtensionsClientInputs
		}
	})) as PublicKeyCredential | null;
	if (!credential) return { ok: false as const, code: 'prf_unsupported' };
	const enabled = (credential.getClientExtensionResults() as PrfResults).prf?.enabled;
	if (enabled === false) return { ok: false as const, code: 'prf_unsupported' };
	const salt = randomBytes(32);
	const id = new Uint8Array(credential.rawId);
	const secret = await prf(new Map([[base64url(id), salt]]));
	if (!secret) return { ok: false as const, code: 'prf_unsupported' };
	const wrapped = await wrapKey(key, await secretKey(secret.bytes, 'passkey'));
	return addUnlock(
		'passkey',
		{ credential_id: base64url(id), salt: toBase64(salt) },
		wrapped,
		label
	);
}

/** Asks one of the credentials for its PRF output with its salt. */
async function prf(salts: Map<string, Uint8Array<ArrayBuffer>>) {
	const assertion = (await navigator.credentials.get({
		publicKey: {
			challenge: randomBytes(32),
			allowCredentials: [...salts.keys()].map((id) => ({
				type: 'public-key' as const,
				id: fromBase64url(id)
			})),
			userVerification: 'required',
			// The DOM types do not know evalByCredential yet.
			extensions: {
				prf: {
					evalByCredential: Object.fromEntries(
						[...salts].map(([id, salt]) => [id, { first: salt }])
					)
				}
			} as unknown as AuthenticationExtensionsClientInputs
		}
	})) as PublicKeyCredential | null;
	const first = (assertion?.getClientExtensionResults() as PrfResults | undefined)?.prf?.results
		?.first;
	if (!assertion || !first) return null;
	return { credential: base64url(new Uint8Array(assertion.rawId)), bytes: new Uint8Array(first) };
}

/** The vault key through one of the registered passkeys, or null. */
export async function unlockWithPasskey(vault: StoredVault) {
	const passkeys = vault.unlocks.filter((unlock) => unlock.kind === 'passkey');
	if (passkeys.length === 0) return null;
	const salts = new Map(
		passkeys.map((unlock) => [
			String(unlock.params.credential_id),
			fromBase64(String(unlock.params.salt))
		])
	);
	const secret = await prf(salts);
	const unlock = passkeys.find((u) => u.params.credential_id === secret?.credential);
	if (!secret || !unlock) return null;
	try {
		return await unwrapKey(
			fromBase64(unlock.wrapped_key),
			await secretKey(secret.bytes, 'passkey')
		);
	} catch {
		return null;
	}
}

export async function readEntries(key: CryptoKey, vault: StoredVault): Promise<Entry[]> {
	return Promise.all(
		vault.entries.map(async ({ id, nonce, ciphertext }) => {
			try {
				const content = (await open(
					key,
					id,
					fromBase64(nonce),
					fromBase64(ciphertext)
				)) as EntryContent;
				return { id, content };
			} catch {
				return { id, content: null };
			}
		})
	);
}

/** The owner's picks (#81); none if there are none yet or they do not open. */
export async function readPicks(key: CryptoKey, vault: StoredVault): Promise<Pick[]> {
	if (!vault.search) return [];
	try {
		const content = (await open(
			key,
			SEARCH_ID,
			fromBase64(vault.search.nonce),
			fromBase64(vault.search.ciphertext)
		)) as { picks?: unknown };
		return Array.isArray(content.picks) ? (content.picks as Pick[]) : [];
	} catch {
		return [];
	}
}

/** Seals the picks and stores them in place of the last ones. */
export async function savePicks(key: CryptoKey, picks: Pick[]) {
	const { nonce, ciphertext } = await seal(key, SEARCH_ID, { picks });
	return api(
		'PUT',
		'/api/personal/search',
		{ nonce: toBase64(nonce), ciphertext: toBase64(ciphertext) },
		lasting
	);
}

/** Encrypts and stores an entry; a new one gets a new ID. */
export async function saveEntry(key: CryptoKey, id: string | null, content: EntryContent) {
	const entryId = id ?? globalThis.crypto.randomUUID();
	const { nonce, ciphertext } = await seal(key, entryId, content);
	const result = await api('PUT', `/api/personal/entries/${entryId}`, {
		nonce: toBase64(nonce),
		ciphertext: toBase64(ciphertext)
	});
	return result.ok ? { ok: true as const, id: entryId } : result;
}
