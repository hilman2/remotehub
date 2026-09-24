import { describe, expect, it } from 'vitest';
import de from '../../../messages/de.json';
import en from '../../../messages/en.json';
import { locales } from '$lib/i18n';
import { ERROR_CODES, errorMessage, readProblem } from './errors';

describe('error messages', () => {
	it('exist for every error code in every locale', () => {
		for (const catalog of [en, de]) {
			for (const code of ERROR_CODES) {
				expect(Object.keys(catalog), code).toContain(`error_${code}`);
			}
		}
	});

	it('render for every error code in every locale', () => {
		for (const locale of locales) {
			for (const code of ERROR_CODES) {
				const text = errorMessage(code, locale);
				expect(text, `${locale}: ${code}`).not.toBe('');
				expect(text, `${locale}: ${code}`).not.toContain('error_');
			}
		}
	});

	it('fall back to a generic text for codes of a newer server', () => {
		expect(errorMessage('something_new', 'en')).toContain('something_new');
		expect(errorMessage('something_new', 'de')).toContain('something_new');
	});
});

describe('readProblem', () => {
	it('reads a problem response', async () => {
		const response = new Response(
			JSON.stringify({ type: 'x', title: 'Not found', status: 404, code: 'not_found' }),
			{ status: 404 }
		);
		expect((await readProblem(response))?.code).toBe('not_found');
	});

	it('returns null for other bodies', async () => {
		expect(await readProblem(new Response('Bad Gateway', { status: 502 }))).toBeNull();
		expect(await readProblem(new Response('{"status":"ok"}'))).toBeNull();
	});
});
