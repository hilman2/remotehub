/**
 * TOTP codes of vault entries (#100), as an authenticator app shows them:
 * RFC 6238 with the parameters of an `otpauth://` URI, or a bare base32
 * secret with the usual ones (SHA-1, six digits, 30 seconds).
 */

export interface TotpParams {
	secret: Uint8Array<ArrayBuffer>;
	algorithm: 'SHA-1' | 'SHA-256' | 'SHA-512';
	digits: number;
	period: number;
}

const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

function base32(text: string): Uint8Array<ArrayBuffer> | null {
	const clean = text.replace(/[\s=-]/g, '').toUpperCase();
	if (!clean || [...clean].some((c) => !BASE32.includes(c))) return null;
	const bytes: number[] = [];
	let bits = 0;
	let value = 0;
	for (const char of clean) {
		value = (value << 5) | BASE32.indexOf(char);
		bits += 5;
		if (bits >= 8) {
			bytes.push((value >>> (bits - 8)) & 0xff);
			bits -= 8;
		}
	}
	return new Uint8Array(bytes);
}

/** What a field's value says about a TOTP, or null if it is none. */
export function parseTotp(value: string): TotpParams | null {
	const text = value.trim();
	if (text.toLowerCase().startsWith('otpauth://totp/')) {
		let url: URL;
		try {
			url = new URL(text);
		} catch {
			return null;
		}
		const secret = base32(url.searchParams.get('secret') ?? '');
		if (!secret) return null;
		const algorithm = (url.searchParams.get('algorithm') ?? 'SHA1').toUpperCase();
		const digits = Number(url.searchParams.get('digits') ?? 6);
		const period = Number(url.searchParams.get('period') ?? 30);
		const names = { SHA1: 'SHA-1', SHA256: 'SHA-256', SHA512: 'SHA-512' } as const;
		if (!(algorithm in names) || ![6, 7, 8].includes(digits) || !(period >= 1 && period <= 300))
			return null;
		return { secret, algorithm: names[algorithm as keyof typeof names], digits, period };
	}
	return null;
}

/** Whether a field holds a TOTP: an `otpauth://` URI, or a base32 secret
 * under a name that says so (KeePass' `otp`, `TOTP Seed` and the like). */
export function totpOf(name: string, value: string): TotpParams | null {
	const uri = parseTotp(value);
	if (uri) return uri;
	if (!/otp|totp/i.test(name)) return null;
	const secret = base32(value);
	return secret && secret.length >= 10
		? { secret, algorithm: 'SHA-1', digits: 6, period: 30 }
		: null;
}

/** The code at `unixSeconds`. */
export async function totpCode(params: TotpParams, unixSeconds: number): Promise<string> {
	const counter = new ArrayBuffer(8);
	new DataView(counter).setBigUint64(0, BigInt(Math.floor(unixSeconds / params.period)));
	const key = await crypto.subtle.importKey(
		'raw',
		params.secret,
		{ name: 'HMAC', hash: params.algorithm },
		false,
		['sign']
	);
	const digest = new Uint8Array(await crypto.subtle.sign('HMAC', key, counter));
	const offset = digest[digest.length - 1] & 0x0f;
	const value =
		(((digest[offset] & 0x7f) << 24) |
			(digest[offset + 1] << 16) |
			(digest[offset + 2] << 8) |
			digest[offset + 3]) %
		10 ** params.digits;
	return value.toString().padStart(params.digits, '0');
}
