import { wordlist } from '@scure/bip39/wordlists/english.js';
import { describe, expect, it } from 'vitest';
import { BUILT_IN, generate, strength, type GeneratorSettings } from './generate';

const many = (settings: GeneratorSettings, count = 200) =>
	Array.from({ length: count }, () => generate(settings));

describe('generated passwords', () => {
	it('are long, varied and without look-alikes by default', () => {
		const passwords = many(BUILT_IN);
		expect(new Set(passwords).size).toBe(200);
		for (const password of passwords) {
			expect(password).toHaveLength(20);
			expect(password).toMatch(/[a-z]/);
			expect(password).toMatch(/[A-Z]/);
			expect(password).toMatch(/[2-9]/);
			expect(password).toMatch(/[-_.!?#%+=]/);
			expect(password).not.toMatch(/[0O1lI]/);
		}
	});

	it('hold only the sets chosen, each of them, at the length asked', () => {
		const digits = { ...BUILT_IN, lower: false, upper: false, symbols: false, length: 8 };
		for (const password of many(digits)) expect(password).toMatch(/^[2-9]{8}$/);
		const letters = { ...BUILT_IN, digits: false, symbols: false, length: 128 };
		for (const password of many(letters, 50)) {
			expect(password).toMatch(/^[a-zA-Z]{128}$/);
			expect(password).toMatch(/[a-z]/);
			expect(password).toMatch(/[A-Z]/);
		}
	});

	it('take look-alikes when allowed', () => {
		const digits = { ...BUILT_IN, lower: false, upper: false, symbols: false, look_alikes: true };
		expect(many(digits).join('')).toMatch(/[01]/);
	});

	it('are empty when no characters are chosen', () => {
		const none = { ...BUILT_IN, lower: false, upper: false, digits: false, symbols: false };
		expect(generate(none)).toBe('');
		expect(strength(none)).toBe(0);
	});
});

describe('generated passphrases', () => {
	it('are words of the list, as many as asked, joined by the separator', () => {
		const settings: GeneratorSettings = {
			...BUILT_IN,
			kind: 'passphrase',
			words: 5,
			separator: '.'
		};
		const phrases = many(settings);
		expect(new Set(phrases).size).toBe(200);
		for (const phrase of phrases) {
			const words = phrase.split('.');
			expect(words).toHaveLength(5);
			for (const word of words) expect(wordlist).toContain(word);
		}
		const joined = generate({ ...settings, separator: '' });
		expect(joined).toMatch(/^[a-z]+$/);
	});
});

describe('strength', () => {
	it('counts the bits of the choice made', () => {
		// 20 characters from 25 + 24 + 8 + 9: the four sets without look-alikes.
		expect(strength(BUILT_IN)).toBe(Math.floor(20 * Math.log2(66)));
		// 2048 words hold 11 bits each.
		expect(strength({ ...BUILT_IN, kind: 'passphrase', words: 7 })).toBe(77);
	});
});
