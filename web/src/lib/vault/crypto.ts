/**
 * End-to-end encryption of the personal vault, scheme `e2e_user_v1`
 * (ADR 0004, crates/server/src/api/personal.rs). Everything happens here in
 * the browser with WebCrypto; the server only ever sees what `seal` and
 * `wrapKey` return.
 *
 * - One random vault key (AES-256-GCM) encrypts every entry, with a fresh
 *   nonce each time and the scheme and entry ID as associated data, so a
 *   ciphertext cannot be moved to another entry.
 * - The vault key is stored wrapped (AES-KW) once per way to unlock it. The
 *   wrapping key comes from a passkey's PRF output or the recovery key
 *   (HKDF-SHA-256), or from the passphrase (PBKDF2-SHA-256; WebCrypto has
 *   no memory-hard function).
 */

export const SCHEME = 'e2e_user_v1';
export const PBKDF2_ITERATIONS = 600_000;
const RECOVERY_BYTES = 20;
const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

const subtle = () => globalThis.crypto.subtle;
const encoder = new TextEncoder();

export function randomBytes(length: number): Uint8Array<ArrayBuffer> {
	return globalThis.crypto.getRandomValues(new Uint8Array(length));
}

/** A new vault key; extractable only so that it can be wrapped. */
export function newVaultKey(): Promise<CryptoKey> {
	return subtle().generateKey({ name: 'AES-GCM', length: 256 }, true, ['encrypt', 'decrypt']);
}

export async function wrapKey(vaultKey: CryptoKey, wrapping: CryptoKey): Promise<Uint8Array> {
	return new Uint8Array(await subtle().wrapKey('raw', vaultKey, wrapping, 'AES-KW'));
}

/** The vault key; throws if `wrapping` is not the key it was wrapped with. */
export function unwrapKey(
	wrapped: Uint8Array<ArrayBuffer>,
	wrapping: CryptoKey
): Promise<CryptoKey> {
	return subtle().unwrapKey('raw', wrapped, wrapping, 'AES-KW', { name: 'AES-GCM' }, true, [
		'encrypt',
		'decrypt'
	]);
}

export async function passphraseKey(
	passphrase: string,
	salt: Uint8Array<ArrayBuffer>,
	iterations = PBKDF2_ITERATIONS
): Promise<CryptoKey> {
	const material = await subtle().importKey('raw', encoder.encode(passphrase), 'PBKDF2', false, [
		'deriveKey'
	]);
	return subtle().deriveKey(
		{ name: 'PBKDF2', hash: 'SHA-256', salt, iterations },
		material,
		{ name: 'AES-KW', length: 256 },
		false,
		['wrapKey', 'unwrapKey']
	);
}

/** A wrapping key from high-entropy secret bytes: a PRF output or the recovery key. */
export async function secretKey(
	secret: Uint8Array<ArrayBuffer>,
	purpose: string
): Promise<CryptoKey> {
	const material = await subtle().importKey('raw', secret, 'HKDF', false, ['deriveKey']);
	return subtle().deriveKey(
		{
			name: 'HKDF',
			hash: 'SHA-256',
			salt: new Uint8Array(32),
			info: encoder.encode(`${SCHEME} ${purpose}`)
		},
		material,
		{ name: 'AES-KW', length: 256 },
		false,
		['wrapKey', 'unwrapKey']
	);
}

function aad(entryId: string): Uint8Array<ArrayBuffer> {
	return encoder.encode(`${SCHEME}\n${entryId}`);
}

export async function seal(
	vaultKey: CryptoKey,
	entryId: string,
	content: unknown
): Promise<{ nonce: Uint8Array<ArrayBuffer>; ciphertext: Uint8Array<ArrayBuffer> }> {
	const nonce = randomBytes(12);
	const plaintext = encoder.encode(JSON.stringify(content));
	const ciphertext = await subtle().encrypt(
		{ name: 'AES-GCM', iv: nonce, additionalData: aad(entryId) },
		vaultKey,
		plaintext
	);
	return { nonce, ciphertext: new Uint8Array(ciphertext) };
}

/** The entry's content; throws if the key, the nonce or the entry ID do not fit. */
export async function open(
	vaultKey: CryptoKey,
	entryId: string,
	nonce: Uint8Array<ArrayBuffer>,
	ciphertext: Uint8Array<ArrayBuffer>
): Promise<unknown> {
	const plaintext = await subtle().decrypt(
		{ name: 'AES-GCM', iv: nonce, additionalData: aad(entryId) },
		vaultKey,
		ciphertext
	);
	return JSON.parse(new TextDecoder().decode(plaintext));
}

/** A recovery key: random bytes, and as text in groups of four (`ABCD-EFGH-…`). */
export function newRecoveryKey(): { bytes: Uint8Array<ArrayBuffer>; text: string } {
	const bytes = randomBytes(RECOVERY_BYTES);
	let bits = '';
	for (const byte of bytes) bits += byte.toString(2).padStart(8, '0');
	let text = '';
	for (let i = 0; i < bits.length; i += 5) text += BASE32[parseInt(bits.slice(i, i + 5), 2)];
	return { bytes, text: text.match(/.{4}/g)!.join('-') };
}

/** The bytes of a recovery key as typed: any case, with or without dashes and spaces; `null` if it is none. */
export function parseRecoveryKey(text: string): Uint8Array<ArrayBuffer> | null {
	const clean = text.toUpperCase().replace(/[\s-]/g, '');
	if (clean.length !== (RECOVERY_BYTES * 8) / 5 || [...clean].some((c) => !BASE32.includes(c))) {
		return null;
	}
	const bits = [...clean].map((c) => BASE32.indexOf(c).toString(2).padStart(5, '0')).join('');
	const bytes = new Uint8Array(RECOVERY_BYTES);
	for (let i = 0; i < RECOVERY_BYTES; i++) bytes[i] = parseInt(bits.slice(i * 8, i * 8 + 8), 2);
	return bytes;
}

export function toBase64(bytes: Uint8Array): string {
	let text = '';
	for (const byte of bytes) text += String.fromCharCode(byte);
	return btoa(text);
}

export function fromBase64(text: string): Uint8Array<ArrayBuffer> {
	return Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
}
