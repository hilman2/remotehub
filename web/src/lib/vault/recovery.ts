/**
 * The organisation recovery key and the recovery of personal vaults (#95,
 * ADR 0009; crates/server/src/api/recovery.rs). The private key is made,
 * sealed and used here in the browser; the server sees the public key only.
 */
import { api, type ApiResult } from '$lib/api/client';
import {
	fromBase64,
	importOrganisationKey,
	newOrganisationKey,
	newRecoveryKey,
	openKeyFile,
	parsePrivateKey,
	privateKeyText,
	sealKeyFile,
	secretKey,
	toBase64,
	unwrapForOrganisation,
	wrapKey,
	type KeyFile
} from './crypto';
import { readEntries, type Entry, type StoredVault } from './vault';

export interface RecoveryKey {
	id: string;
	public_key: string;
	created_by_name: string;
	/** RFC 3339, UTC */
	created_at: string;
	/** How many vaults are wrapped for it. */
	vaults: number;
	/** Whether it holds the server's master key in use (#96). */
	master_key: boolean;
}

export interface CoveredVault {
	user_id: string;
	display_name: string;
	username: string;
	/** The key the vault is wrapped for; null if it is not. */
	key_id: string | null;
}

export interface Keys {
	/** Newest first; vaults are wrapped for the first. */
	keys: RecoveryKey[];
	vaults: CoveredVault[];
}

export type RecoveryKind = 'passphrase' | 'handover';
export type RecoveryStatus = 'pending' | 'approved' | 'expired' | 'completed';

export interface Recovery {
	id: string;
	user_id: string;
	user_name: string;
	kind: RecoveryKind;
	reason: string;
	requester_name: string;
	approver_name: string | null;
	/** `approved` holds for a day after the approval. */
	status: RecoveryStatus;
	created_at: string;
	/** Whether the caller asked for it, and so may carry it out. */
	mine: boolean;
}

export const loadKeys = () => api<Keys>('GET', '/api/recovery-keys');
export const deleteKey = (id: string) => api('DELETE', `/api/recovery-keys/${id}`);
export const loadRecoveries = () => api<Recovery[]>('GET', '/api/vault-recoveries');
export const askRecovery = (user_id: string, kind: RecoveryKind, reason: string) =>
	api<{ id: string }>('POST', '/api/vault-recoveries', { user_id, kind, reason });
export const approveRecovery = (id: string) => api('POST', `/api/vault-recoveries/${id}/approve`);
export const cancelRecovery = (id: string) => api('DELETE', `/api/vault-recoveries/${id}`);

/**
 * Makes a key pair and stores its public key. Returns the private key twice,
 * as a file sealed with `passphrase` and as printable text; neither is kept
 * anywhere else.
 */
export async function createKey(
	passphrase: string
): Promise<ApiResult<{ file: KeyFile; text: string }>> {
	const { publicKey, privateKey } = await newOrganisationKey();
	const created = await api<{ id: string }>('POST', '/api/recovery-keys', {
		public_key: toBase64(publicKey)
	});
	if (!created.ok) return created;
	const file = await sealKeyFile(created.data.id, publicKey, privateKey, passphrase);
	return { ok: true, data: { file, text: privateKeyText(privateKey) } };
}

/** How the private key is at hand: the file and its passphrase, or the printed text. */
export type PrivateKeyInput = { file: File; passphrase: string } | { text: string };

/** The private key's scalar; null if the file, passphrase or text is not one. */
export async function readPrivateKey(input: PrivateKeyInput): Promise<Uint8Array | null> {
	if ('text' in input) return parsePrivateKey(input.text);
	try {
		return await openKeyFile(JSON.parse(await input.file.text()) as KeyFile, input.passphrase);
	} catch {
		return null;
	}
}

/** A vault of an approved recovery, opened. */
export interface OpenedVault {
	key: CryptoKey;
	entries: Entry[];
}

/**
 * Opens the vault of an approved recovery with the organisation's private
 * key. The server records that it was opened. Null if the key is not the one
 * the vault is wrapped for.
 */
export async function openVault(
	id: string,
	privateKey: Uint8Array
): Promise<ApiResult<OpenedVault | null>> {
	const sealed = await api<{
		unlock: { params: Record<string, unknown>; wrapped_key: string; public_key: string };
		entries: StoredVault['entries'];
	}>('GET', `/api/vault-recoveries/${id}/vault`);
	if (!sealed.ok) return sealed;
	const { unlock, entries } = sealed.data;
	try {
		const own = await importOrganisationKey(privateKey, fromBase64(unlock.public_key));
		const key = await unwrapForOrganisation(
			fromBase64(unlock.wrapped_key),
			fromBase64(String(unlock.params.ephemeral)),
			own
		);
		return { ok: true, data: { key, entries: await readEntries(key, { entries }) } };
	} catch {
		return { ok: true, data: null };
	}
}

/** Where the files of the vault being recovered are read from. */
export const attachmentsOf = (id: string) => `/api/vault-recoveries/${id}/attachments`;

/**
 * Ends a recovery of a forgotten passphrase: a new recovery key, used once,
 * replaces the owner's. Returns its text for the owner.
 */
export async function completeWithOneTimeKey(
	id: string,
	vaultKey: CryptoKey
): Promise<ApiResult<string>> {
	const recovery = newRecoveryKey();
	const wrapped = await wrapKey(vaultKey, await secretKey(recovery.bytes, 'recovery'));
	const done = await api('POST', `/api/vault-recoveries/${id}/complete`, {
		wrapped_key: toBase64(wrapped)
	});
	return done.ok ? { ok: true, data: recovery.text } : done;
}

/** Ends a hand-over, once the entries are in the shared folder. */
export const completeHandover = (id: string) =>
	api('POST', `/api/vault-recoveries/${id}/complete`, {});
