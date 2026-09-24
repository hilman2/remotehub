import { describe, expect, it } from 'vitest';
import { api, NETWORK_ERROR } from './client';

describe('api', () => {
	it('sends JSON and returns the data', async () => {
		let sent: RequestInit | undefined;
		const fetcher: typeof fetch = async (_, init) => {
			sent = init;
			return new Response(JSON.stringify({ username: 'alice' }), { status: 200 });
		};
		const result = await api('POST', '/api/session', { username: 'alice' }, fetcher);
		expect(result).toEqual({ ok: true, data: { username: 'alice' } });
		expect(sent?.headers).toMatchObject({ 'content-type': 'application/json' });
		expect(sent?.body).toBe('{"username":"alice"}');
	});

	it('returns the problem code of errors', async () => {
		const fetcher: typeof fetch = async () =>
			new Response(
				JSON.stringify({
					type: 'x',
					title: 'x',
					status: 429,
					code: 'too_many_attempts',
					params: { retry_after_seconds: 60 }
				}),
				{ status: 429 }
			);
		expect(await api('POST', '/api/session', {}, fetcher)).toEqual({
			ok: false,
			status: 429,
			code: 'too_many_attempts',
			params: { retry_after_seconds: 60 }
		});
	});

	it('handles empty answers and network failures', async () => {
		const empty: typeof fetch = async () => new Response(null, { status: 204 });
		expect(await api('DELETE', '/api/session', undefined, empty)).toEqual({
			ok: true,
			data: undefined
		});
		const offline: typeof fetch = async () => {
			throw new TypeError('offline');
		};
		expect((await api('GET', '/api/session', undefined, offline)).ok).toBe(false);
		const result = await api('GET', '/api/session', undefined, offline);
		expect(!result.ok && result.code).toBe(NETWORK_ERROR);
	});
});
