import { describe, expect, it } from 'vitest';
import { parseTotp, totpCode, totpOf } from './totp';

/** "12345678901234567890" in base32, the secret of RFC 6238's SHA-1 vectors. */
const RFC_SECRET = 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ';

describe('TOTP codes', () => {
	it('match RFC 6238, appendix B', async () => {
		const params = parseTotp(`otpauth://totp/x?secret=${RFC_SECRET}&digits=8`)!;
		for (const [time, code] of [
			[59, '94287082'],
			[1111111109, '07081804'],
			[1234567890, '89005924'],
			[2000000000, '69279037']
		] as const) {
			expect(await totpCode(params, time)).toBe(code);
		}
	});

	it('read URIs and secrets under an OTP name, nothing else', () => {
		const uri = parseTotp(
			`otpauth://totp/remotehub:alice?secret=${RFC_SECRET}&algorithm=SHA256&period=60`
		);
		expect(uri).toMatchObject({ algorithm: 'SHA-256', digits: 6, period: 60 });
		expect(totpOf('otp', RFC_SECRET.toLowerCase())).toMatchObject({ digits: 6, period: 30 });
		expect(totpOf('Notes', RFC_SECRET)).toBeNull();
		expect(totpOf('otp', 'not a secret!')).toBeNull();
		expect(parseTotp(`otpauth://totp/x?secret=${RFC_SECRET}&digits=12`)).toBeNull();
		expect(parseTotp('otpauth://hotp/x?secret=AAAA')).toBeNull();
	});
});
