import { describe, expect, it } from 'vitest';
import {
	fromBase64,
	newRecoveryKey,
	newVaultKey,
	open,
	parseRecoveryKey,
	passphraseKey,
	randomBytes,
	openFile,
	seal,
	sealFile,
	secretKey,
	toBase64,
	unwrapKey,
	wrapKey
} from './crypto';

const ENTRY = '7c9e6679-7425-40de-944b-e07fc1f90ae7';
const OTHER = '2f1b8a9e-6d0c-4f37-9c5e-1a2b3c4d5e6f';

describe('personal vault encryption', () => {
	it('opens what it sealed, and only for the same entry', async () => {
		const key = await newVaultKey();
		const content = { title: 'Router', password: 'Pässwörd 😀' };
		const { nonce, ciphertext } = await seal(key, ENTRY, content);
		expect(await open(key, ENTRY, nonce, ciphertext)).toEqual(content);
		// The same ciphertext under another entry's ID does not open.
		await expect(open(key, OTHER, nonce, ciphertext)).rejects.toThrow();
		// Nor with another key, nor tampered with.
		await expect(open(await newVaultKey(), ENTRY, nonce, ciphertext)).rejects.toThrow();
		const tampered = ciphertext.slice();
		tampered[0] ^= 1;
		await expect(open(key, ENTRY, nonce, tampered)).rejects.toThrow();
		// Nothing of the content shows in the ciphertext.
		expect(new TextDecoder().decode(ciphertext)).not.toContain('Router');
	});

	it('keeps files apart from entries', async () => {
		const key = await newVaultKey();
		const bytes = new Uint8Array(200_000).map((_, i) => i % 251);
		const { nonce, ciphertext } = await sealFile(key, ENTRY, bytes);
		expect(await openFile(key, ENTRY, nonce, ciphertext)).toEqual(bytes);
		await expect(openFile(key, OTHER, nonce, ciphertext)).rejects.toThrow();
		// A file does not open as an entry of the same ID, nor the other way.
		await expect(open(key, ENTRY, nonce, ciphertext)).rejects.toThrow();
		const entry = await seal(key, ENTRY, { title: 'x' });
		await expect(openFile(key, ENTRY, entry.nonce, entry.ciphertext)).rejects.toThrow();
		// Large files go through base64 and back.
		expect(fromBase64(toBase64(ciphertext))).toEqual(ciphertext);
	});

	it('uses a new nonce every time', async () => {
		const key = await newVaultKey();
		const first = await seal(key, ENTRY, { a: 1 });
		const second = await seal(key, ENTRY, { a: 1 });
		expect(toBase64(first.nonce)).not.toBe(toBase64(second.nonce));
		expect(toBase64(first.ciphertext)).not.toBe(toBase64(second.ciphertext));
	});

	it('wraps the vault key so that only the right passphrase unwraps it', async () => {
		const vaultKey = await newVaultKey();
		const salt = randomBytes(16);
		// Few iterations: the test checks the construction, not the cost.
		const wrapped = await wrapKey(vaultKey, await passphraseKey('correct horse', salt, 1000));
		const unwrapped = await unwrapKey(
			new Uint8Array(wrapped),
			await passphraseKey('correct horse', salt, 1000)
		);
		const { nonce, ciphertext } = await seal(vaultKey, ENTRY, 'secret');
		expect(await open(unwrapped, ENTRY, nonce, ciphertext)).toBe('secret');
		await expect(
			unwrapKey(new Uint8Array(wrapped), await passphraseKey('wrong horse', salt, 1000))
		).rejects.toThrow();
		await expect(
			unwrapKey(
				new Uint8Array(wrapped),
				await passphraseKey('correct horse', randomBytes(16), 1000)
			)
		).rejects.toThrow();
	});

	it('keeps passkeys and the recovery key apart even for the same secret', async () => {
		const vaultKey = await newVaultKey();
		const secret = randomBytes(32);
		const wrapped = await wrapKey(vaultKey, await secretKey(secret, 'passkey'));
		await unwrapKey(new Uint8Array(wrapped), await secretKey(secret, 'passkey'));
		await expect(
			unwrapKey(new Uint8Array(wrapped), await secretKey(secret, 'recovery'))
		).rejects.toThrow();
	});

	it('reads recovery keys back as people type them', () => {
		const { bytes, text } = newRecoveryKey();
		expect(text).toMatch(/^([A-Z2-7]{4}-){7}[A-Z2-7]{4}$/);
		expect(parseRecoveryKey(text)).toEqual(bytes);
		expect(parseRecoveryKey(` ${text.toLowerCase().replaceAll('-', ' ')} `)).toEqual(bytes);
		expect(parseRecoveryKey(text.slice(0, -1))).toBeNull();
		expect(parseRecoveryKey(text.replace(/.$/, '1'))).toBeNull();
		expect(newRecoveryKey().text).not.toBe(text);
	});

	it('carries bytes through base64', () => {
		const bytes = randomBytes(40);
		expect(fromBase64(toBase64(bytes))).toEqual(bytes);
	});
});
