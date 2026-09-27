/**
 * Passwords and passphrases made in the browser (#100, #194): random from
 * the Web Crypto API, as the generator's settings ask.
 */
import { wordlist } from '@scure/bip39/wordlists/english.js';

/** What the generator makes; the server keeps the organisation's and each user's. */
export interface GeneratorSettings {
	kind: 'password' | 'passphrase';
	/** Characters of a password. */
	length: number;
	lower: boolean;
	upper: boolean;
	digits: boolean;
	symbols: boolean;
	/** Whether a password may hold characters easily taken for others: 0 O 1 l I. */
	look_alikes: boolean;
	/** Words of a passphrase. */
	words: number;
	/** Between the words of a passphrase; at most three characters. */
	separator: string;
}

/** The bounds the server holds settings to. */
export const LENGTH = { min: 8, max: 128 };
export const WORDS = { min: 3, max: 20 };

/** Until an administrator sets the organisation's. */
export const BUILT_IN: GeneratorSettings = {
	kind: 'password',
	length: 20,
	lower: true,
	upper: true,
	digits: true,
	symbols: true,
	look_alikes: false,
	words: 6,
	separator: '-'
};

const SETS = {
	lower: 'abcdefghijklmnopqrstuvwxyz',
	upper: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ',
	digits: '0123456789',
	symbols: '-_.!?#%+='
} as const;
const LOOK_ALIKES = '0O1lI';

/** A uniformly random index below `bound`, without modulo bias. */
function below(bound: number): number {
	const limit = Math.floor(0x1_0000_0000 / bound) * bound;
	const value = new Uint32Array(1);
	do crypto.getRandomValues(value);
	while (value[0] >= limit);
	return value[0] % bound;
}

/** The character sets `settings` choose, each without look-alikes unless allowed. */
function sets(settings: GeneratorSettings): string[] {
	return (Object.keys(SETS) as (keyof typeof SETS)[])
		.filter((set) => settings[set])
		.map((set) =>
			settings.look_alikes
				? SETS[set]
				: [...SETS[set]].filter((c) => !LOOK_ALIKES.includes(c)).join('')
		);
}

/** A password or a passphrase as `settings` ask; empty if they choose no characters. */
export function generate(settings: GeneratorSettings): string {
	if (settings.kind === 'passphrase') {
		return Array.from({ length: settings.words }, () => wordlist[below(wordlist.length)]).join(
			settings.separator
		);
	}
	const chosen = sets(settings);
	const all = chosen.join('');
	if (!all) return '';
	for (;;) {
		const password = Array.from({ length: settings.length }, () => all[below(all.length)]).join('');
		// Most sites want each kind chosen; drawing again is fairer than inserting.
		if (chosen.every((set) => [...password].some((c) => set.includes(c)))) return password;
	}
}

/**
 * About how many bits of chance a result of `settings` holds, as KeePass
 * shows it: what guessing it takes for someone who knows the settings.
 */
export function strength(settings: GeneratorSettings): number {
	if (settings.kind === 'passphrase') {
		return Math.floor(settings.words * Math.log2(wordlist.length));
	}
	const size = sets(settings).join('').length;
	return size ? Math.floor(settings.length * Math.log2(size)) : 0;
}
