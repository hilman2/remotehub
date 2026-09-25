/**
 * Passwords made in the browser (#100): random from the Web Crypto API,
 * without look-alikes (0/O, 1/l/I), with every kind of character in it.
 */

const LOWER = 'abcdefghijkmnopqrstuvwxyz';
const UPPER = 'ABCDEFGHJKLMNPQRSTUVWXYZ';
const DIGITS = '23456789';
const SYMBOLS = '-_.!?#%+=';
const ALL = LOWER + UPPER + DIGITS + SYMBOLS;

/** A uniformly random index below `bound`, without modulo bias. */
function below(bound: number): number {
	const limit = Math.floor(0x1_0000_0000 / bound) * bound;
	const value = new Uint32Array(1);
	do crypto.getRandomValues(value);
	while (value[0] >= limit);
	return value[0] % bound;
}

export function generatePassword(length = 20): string {
	for (;;) {
		const password = Array.from({ length }, () => ALL[below(ALL.length)]).join('');
		// Most sites want each kind; drawing again is fairer than inserting.
		if ([LOWER, UPPER, DIGITS, SYMBOLS].every((set) => [...password].some((c) => set.includes(c))))
			return password;
	}
}
