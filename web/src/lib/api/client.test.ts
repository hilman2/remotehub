import { afterEach, describe, expect, it } from 'vitest';
import { api, guard, NETWORK_ERROR } from './client';

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

describe('the guard', () => {
	const problem = (status: number, code: string) =>
		new Response(JSON.stringify({ type: 'x', title: 'x', status, code, params: {} }), { status });

	/** Answers with `first` once, then with `{}`; counts the requests. */
	function server(first: Response) {
		const seen = { requests: 0 };
		const fetcher: typeof fetch = async () => {
			seen.requests += 1;
			return seen.requests === 1 ? first : new Response('{}', { status: 200 });
		};
		return { seen, fetcher };
	}

	afterEach(() => {
		delete guard.locked;
		delete guard.confirm;
		delete guard.ended;
	});

	it('sends a request again once the locked session is unlocked (#241)', async () => {
		let asked = 0;
		guard.locked = async () => (asked += 1) > 0;
		const { seen, fetcher } = server(problem(401, 'session_locked'));
		expect(await api('GET', '/api/tree', undefined, fetcher)).toEqual({ ok: true, data: {} });
		expect([asked, seen.requests]).toEqual([1, 2]);
	});

	it('sends a request again once confirmed, and not when declined (#242)', async () => {
		guard.confirm = async () => true;
		const confirmed = server(problem(403, 'confirmation_required'));
		expect((await api('POST', '/api/credentials/x/reveal', {}, confirmed.fetcher)).ok).toBe(true);
		guard.confirm = async () => false;
		const declined = server(problem(403, 'confirmation_required'));
		const result = await api('POST', '/api/credentials/x/reveal', {}, declined.fetcher);
		expect(!result.ok && result.code).toBe('confirmation_required');
		expect(declined.seen.requests).toBe(1);
	});

	it('reports a session that is over (#240)', async () => {
		let ended = 0;
		guard.ended = () => (ended += 1);
		const { fetcher } = server(problem(401, 'unauthenticated'));
		await api('GET', '/api/tree', undefined, fetcher);
		expect(ended).toBe(1);
	});
});
