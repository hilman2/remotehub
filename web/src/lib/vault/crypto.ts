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
 * - It is also wrapped for the organisation recovery key (#95), so an
 *   approved recovery can open it with the organisation's private key.
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

/**
 * The associated data of a file (#100): not an entry's, so neither can be
 * passed off as the other.
 */
function fileAad(fileId: string): Uint8Array<ArrayBuffer> {
	return encoder.encode(`${SCHEME}\nfile\n${fileId}`);
}

/** Seals the bytes of a file of the personal vault. */
export async function sealFile(
	vaultKey: CryptoKey,
	fileId: string,
	content: Uint8Array<ArrayBuffer>
): Promise<{ nonce: Uint8Array<ArrayBuffer>; ciphertext: Uint8Array<ArrayBuffer> }> {
	const nonce = randomBytes(12);
	const ciphertext = await subtle().encrypt(
		{ name: 'AES-GCM', iv: nonce, additionalData: fileAad(fileId) },
		vaultKey,
		content
	);
	return { nonce, ciphertext: new Uint8Array(ciphertext) };
}

/** The bytes of a file; throws if key, nonce or ID do not fit. */
export async function openFile(
	vaultKey: CryptoKey,
	fileId: string,
	nonce: Uint8Array<ArrayBuffer>,
	ciphertext: Uint8Array<ArrayBuffer>
): Promise<Uint8Array<ArrayBuffer>> {
	const plaintext = await subtle().decrypt(
		{ name: 'AES-GCM', iv: nonce, additionalData: fileAad(fileId) },
		vaultKey,
		ciphertext
	);
	return new Uint8Array(plaintext);
}

/** Bytes as base32 text in groups of four (`ABCD-EFGH-…`), for typing off paper. */
function toGroups(bytes: Uint8Array): string {
	let bits = '';
	for (const byte of bytes) bits += byte.toString(2).padStart(8, '0');
	bits = bits.padEnd(Math.ceil(bits.length / 5) * 5, '0');
	let text = '';
	for (let i = 0; i < bits.length; i += 5) text += BASE32[parseInt(bits.slice(i, i + 5), 2)];
	return text.match(/.{1,4}/g)!.join('-');
}

/** `length` bytes from text as `toGroups` wrote it, typed in any case, with or without dashes and spaces; `null` if it is none. */
function fromGroups(text: string, length: number): Uint8Array<ArrayBuffer> | null {
	const clean = text.toUpperCase().replace(/[\s-]/g, '');
	if (clean.length !== Math.ceil((length * 8) / 5) || [...clean].some((c) => !BASE32.includes(c))) {
		return null;
	}
	const bits = [...clean].map((c) => BASE32.indexOf(c).toString(2).padStart(5, '0')).join('');
	const bytes = new Uint8Array(length);
	for (let i = 0; i < length; i++) bytes[i] = parseInt(bits.slice(i * 8, i * 8 + 8), 2);
	return bytes;
}

/** A recovery key: random bytes, and as text in groups of four (`ABCD-EFGH-…`). */
export function newRecoveryKey(): { bytes: Uint8Array<ArrayBuffer>; text: string } {
	const bytes = randomBytes(RECOVERY_BYTES);
	return { bytes, text: toGroups(bytes) };
}

/** The bytes of a recovery key as typed; `null` if it is none. */
export function parseRecoveryKey(text: string): Uint8Array<ArrayBuffer> | null {
	return fromGroups(text, RECOVERY_BYTES);
}

// The organisation recovery key (#95, ADR 0009): an ECDH key pair on P-256.
// A vault key is wrapped for its public key with an ephemeral key pair: the
// shared secret goes through HKDF into an AES-KW key.
const ECDH = { name: 'ECDH', namedCurve: 'P-256' } as const;
const SCALAR_BYTES = 32;

export function base64url(bytes: Uint8Array): string {
	return toBase64(bytes).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

export function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
	const padded = text.replaceAll('-', '+').replaceAll('_', '/');
	return fromBase64(padded + '='.repeat((4 - (padded.length % 4)) % 4));
}

/**
 * A new organisation key pair: the public key as an uncompressed point (65
 * bytes) and the private key as its scalar (32 bytes).
 */
export async function newOrganisationKey(): Promise<{
	publicKey: Uint8Array<ArrayBuffer>;
	privateKey: Uint8Array<ArrayBuffer>;
}> {
	const pair = await subtle().generateKey(ECDH, true, ['deriveBits']);
	const publicKey = new Uint8Array(await subtle().exportKey('raw', pair.publicKey));
	const { d } = await subtle().exportKey('jwk', pair.privateKey);
	return { publicKey, privateKey: fromBase64url(d!) };
}

/** The private key as printable text, for a safe. */
export const privateKeyText = (privateKey: Uint8Array) => toGroups(privateKey);

/** The private key's scalar as typed off paper; `null` if it is none. */
export const parsePrivateKey = (text: string) => fromGroups(text, SCALAR_BYTES);

/**
 * The private key for ECDH. WebCrypto takes a private key as JWK only with
 * its public point, so both are needed. A scalar that does not belong to
 * `publicKey` is refused here (Node, Chromium) or unwraps nothing later.
 */
export function importOrganisationKey(
	privateKey: Uint8Array,
	publicKey: Uint8Array
): Promise<CryptoKey> {
	const jwk: JsonWebKey = {
		kty: 'EC',
		crv: 'P-256',
		d: base64url(privateKey),
		x: base64url(publicKey.subarray(1, 33)),
		y: base64url(publicKey.subarray(33, 65))
	};
	return subtle().importKey('jwk', jwk, ECDH, false, ['deriveBits']);
}

async function sharedKey(own: CryptoKey, other: Uint8Array<ArrayBuffer>): Promise<CryptoKey> {
	const point = await subtle().importKey('raw', other, ECDH, false, []);
	const secret = await subtle().deriveBits({ name: 'ECDH', public: point }, own, 256);
	return secretKey(new Uint8Array(secret), 'organisation');
}

/** The vault key wrapped for the organisation's public key, with the ephemeral public key that opens it again. */
export async function wrapForOrganisation(
	vaultKey: CryptoKey,
	publicKey: Uint8Array<ArrayBuffer>
): Promise<{ ephemeral: Uint8Array<ArrayBuffer>; wrapped: Uint8Array }> {
	const pair = await subtle().generateKey(ECDH, true, ['deriveBits']);
	const ephemeral = new Uint8Array(await subtle().exportKey('raw', pair.publicKey));
	const wrapped = await wrapKey(vaultKey, await sharedKey(pair.privateKey, publicKey));
	return { ephemeral, wrapped };
}

/** The vault key; throws if `privateKey` is not the one it was wrapped for. */
export async function unwrapForOrganisation(
	wrapped: Uint8Array<ArrayBuffer>,
	ephemeral: Uint8Array<ArrayBuffer>,
	privateKey: CryptoKey
): Promise<CryptoKey> {
	return unwrapKey(wrapped, await sharedKey(privateKey, ephemeral));
}

/** The private key as a file, sealed with a passphrase of its own. */
export interface KeyFile {
	format: 'remotehub-recovery-key-1';
	key_id: string;
	public_key: string;
	salt: string;
	iterations: number;
	nonce: string;
	ciphertext: string;
}

const keyFileAad = (keyId: string) => encoder.encode(`remotehub recovery key\n${keyId}`);

async function keyFileKey(passphrase: string, salt: Uint8Array<ArrayBuffer>, iterations: number) {
	const material = await subtle().importKey('raw', encoder.encode(passphrase), 'PBKDF2', false, [
		'deriveKey'
	]);
	return subtle().deriveKey(
		{ name: 'PBKDF2', hash: 'SHA-256', salt, iterations },
		material,
		{ name: 'AES-GCM', length: 256 },
		false,
		['encrypt', 'decrypt']
	);
}

export async function sealKeyFile(
	keyId: string,
	publicKey: Uint8Array,
	privateKey: Uint8Array<ArrayBuffer>,
	passphrase: string
): Promise<KeyFile> {
	const salt = randomBytes(16);
	const nonce = randomBytes(12);
	const key = await keyFileKey(passphrase, salt, PBKDF2_ITERATIONS);
	const ciphertext = await subtle().encrypt(
		{ name: 'AES-GCM', iv: nonce, additionalData: keyFileAad(keyId) },
		key,
		privateKey
	);
	return {
		format: 'remotehub-recovery-key-1',
		key_id: keyId,
		public_key: toBase64(publicKey),
		salt: toBase64(salt),
		iterations: PBKDF2_ITERATIONS,
		nonce: toBase64(nonce),
		ciphertext: toBase64(new Uint8Array(ciphertext))
	};
}

/** The private key's scalar; throws if the passphrase or the file is wrong. */
export async function openKeyFile(
	file: KeyFile,
	passphrase: string
): Promise<Uint8Array<ArrayBuffer>> {
	if (file.format !== 'remotehub-recovery-key-1') throw new Error('not a recovery key file');
	const key = await keyFileKey(passphrase, fromBase64(file.salt), file.iterations);
	const scalar = await subtle().decrypt(
		{ name: 'AES-GCM', iv: fromBase64(file.nonce), additionalData: keyFileAad(file.key_id) },
		key,
		fromBase64(file.ciphertext)
	);
	return new Uint8Array(scalar);
}

export function toBase64(bytes: Uint8Array): string {
	// In pieces: files of megabytes would make one string of each byte slow.
	const pieces: string[] = [];
	for (let at = 0; at < bytes.length; at += 0x8000) {
		pieces.push(String.fromCharCode(...bytes.subarray(at, at + 0x8000)));
	}
	return btoa(pieces.join(''));
}

export function fromBase64(text: string): Uint8Array<ArrayBuffer> {
	return Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
}
