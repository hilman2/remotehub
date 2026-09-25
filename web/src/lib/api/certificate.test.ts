import { describe, expect, it } from 'vitest';
import { expiryWarning, type CertificateInfo } from './certificate';

const DAY = 24 * 3600 * 1000;
const now = Date.UTC(2026, 8, 25);

/** A certificate that runs out `days` from `now`. */
const runningOut = (days: number): CertificateInfo => ({
	subject: 'CN=remotehub.example.com',
	issuer: 'CN=Example CA',
	names: ['remotehub.example.com'],
	not_before: 0,
	not_after: (now + days * DAY) / 1000,
	fingerprint: 'AA'
});

describe('expiryWarning', () => {
	it('warns 30 days before the end, again at 7, and once it has run out', () => {
		expect(expiryWarning(runningOut(31), now)).toBeNull();
		expect(expiryWarning(runningOut(29), now)).toBe(30);
		expect(expiryWarning(runningOut(8), now)).toBe(30);
		expect(expiryWarning(runningOut(6), now)).toBe(7);
		expect(expiryWarning(runningOut(-1), now)).toBe(7);
	});
});
