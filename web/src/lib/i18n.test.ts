import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import de from '../../messages/de.json';
import en from '../../messages/en.json';
import { formatLocale } from './i18n';

/** A message is a string, or a plural/select object of the inlang format. */
type Catalog = Record<string, unknown>;

const CATALOGS: Record<string, Catalog> = { en, de };

/**
 * Keys whose German text may equal the English one because the word is the
 * same in both languages (technical terms, protocol names). Everything else
 * that is identical is most likely a forgotten translation.
 */
const IDENTICAL_IN_GERMAN = new Set<string>([
	'footer_version',
	'field_name',
	'field_port',
	'protocol_ssh',
	'protocol_vnc',
	'credential_version',
	'field_passphrase',
	'vault_kind_passkey',
	'vault_kind_passphrase',
	'sign_in_code',
	'users_code_link',
	'role_administrator'
]);

/** Every message as text; plural and select variants as their JSON. */
const messages = (catalog: Catalog): [string, string][] =>
	Object.entries(catalog)
		.filter(([key]) => key !== '$schema')
		.map(([key, value]) => [key, typeof value === 'string' ? value : JSON.stringify(value)]);

/** Placeholders like {name} in the inlang message format. */
function placeholders(text: string): string[] {
	return [...new Set([...text.matchAll(/\{\s*(\w+)\s*\}/g)].map((m) => m[1]))].sort();
}

const textOf = (catalog: Catalog, key: string) => new Map(messages(catalog)).get(key) ?? '';

describe('message catalogs', () => {
	it('name every key once', () => {
		// JSON.parse keeps the last of two equal keys without a word, so the
		// raw files are read.
		for (const locale of Object.keys(CATALOGS)) {
			const raw = readFileSync(new URL(`../../messages/${locale}.json`, import.meta.url), 'utf8');
			const keys = [...raw.matchAll(/^\t"([^"]+)":/gm)].map((match) => match[1]);
			const twice = keys.filter((key, index) => keys.indexOf(key) !== index);
			expect(twice, locale).toEqual([]);
		}
	});

	it('have the same keys in every locale', () => {
		const keys = (catalog: Catalog) => Object.keys(catalog).sort();
		for (const catalog of Object.values(CATALOGS)) {
			expect(keys(catalog)).toEqual(keys(en));
		}
	});

	it('have no empty messages', () => {
		for (const [locale, catalog] of Object.entries(CATALOGS)) {
			for (const [key, text] of messages(catalog)) {
				expect(text.trim(), `${locale}: ${key}`).not.toBe('');
			}
		}
	});

	it('use the same placeholders for a key in every locale', () => {
		for (const [key, text] of messages(en)) {
			for (const [locale, catalog] of Object.entries(CATALOGS)) {
				expect(placeholders(textOf(catalog, key)), `${locale}: ${key}`).toEqual(placeholders(text));
			}
		}
	});

	it('translate every German text that is not allow-listed', () => {
		const identical = messages(de)
			.filter(([key, text]) => text === textOf(en, key) && !IDENTICAL_IN_GERMAN.has(key))
			.map(([key]) => key);
		expect(identical, 'translate them or add them to IDENTICAL_IN_GERMAN').toEqual([]);
	});

	it('allow-list only keys that exist and are really identical', () => {
		for (const key of IDENTICAL_IN_GERMAN) {
			expect(textOf(de, key), key).not.toBe('');
			expect(textOf(de, key), key).toBe(textOf(en, key));
		}
	});
});

describe('placeholders', () => {
	it('finds every named placeholder once', () => {
		expect(placeholders('{count} devices in {folder}, {count} online')).toEqual([
			'count',
			'folder'
		]);
		expect(placeholders('no placeholders')).toEqual([]);
	});
});

describe('formatLocale', () => {
	it('prefers the browser’s regional variant of the UI language', () => {
		expect(formatLocale('en', ['de-DE', 'en-US'])).toBe('en-US');
		expect(formatLocale('de', ['de-AT', 'en'])).toBe('de-AT');
	});

	it('falls back to a default region', () => {
		expect(formatLocale('en', ['en'])).toBe('en-GB');
		expect(formatLocale('de', [])).toBe('de-DE');
	});
});
