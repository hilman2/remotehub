import { describe, expect, it } from 'vitest';
import { fetchServerState } from './health';

const respond =
	(status: number, body: unknown): typeof fetch =>
	async () =>
		new Response(JSON.stringify(body), { status });

describe('fetchServerState', () => {
	it('reports a healthy server with its version', async () => {
		const state = await fetchServerState(
			respond(200, { status: 'ok', version: '0.1.0', database: 'ok' })
		);
		expect(state).toEqual({ kind: 'ok', version: '0.1.0' });
	});

	it('reports a missing database from a 503', async () => {
		const state = await fetchServerState(
			respond(503, { status: 'unavailable', version: '0.1.0', database: 'unavailable' })
		);
		expect(state).toEqual({ kind: 'database_unavailable', version: '0.1.0' });
	});

	it('reports an unreachable server on errors and other status codes', async () => {
		expect(await fetchServerState(respond(502, 'Bad Gateway'))).toEqual({ kind: 'unreachable' });
		const failing: typeof fetch = async () => {
			throw new TypeError('network');
		};
		expect(await fetchServerState(failing)).toEqual({ kind: 'unreachable' });
	});
});
