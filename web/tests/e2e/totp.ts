import { createHmac } from 'node:crypto';

/** RFC 6238 with SHA-1 and six digits, as authenticator apps compute it. */
export function totp(secret: string, step: number): string {
	const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
	let bits = '';
	for (const char of secret.replace(/=+$/, '').toUpperCase()) {
		bits += alphabet.indexOf(char).toString(2).padStart(5, '0');
	}
	const key = Buffer.from(bits.match(/.{8}/g)!.map((byte) => parseInt(byte, 2)));
	const counter = Buffer.alloc(8);
	counter.writeBigUInt64BE(BigInt(step));
	const hmac = createHmac('sha1', key).update(counter).digest();
	const offset = hmac[hmac.length - 1] & 0xf;
	const value = (hmac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000;
	return value.toString().padStart(6, '0');
}

/** The current TOTP time step. */
export const step = () => Math.floor(Date.now() / 30_000);
