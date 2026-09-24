/** `GET /api/health` of the server (crates/server/src/api/health.rs). */
export interface Health {
	status: 'ok' | 'unavailable';
	version: string;
	database: 'ok' | 'unavailable';
}

export type ServerState =
	| { kind: 'ok'; version: string }
	| { kind: 'database_unavailable'; version: string }
	| { kind: 'unreachable' };

/** Asks the server how it is; never throws. */
export async function fetchServerState(fetcher: typeof fetch = fetch): Promise<ServerState> {
	try {
		const response = await fetcher('/api/health', { headers: { accept: 'application/json' } });
		if (!response.ok && response.status !== 503) return { kind: 'unreachable' };
		const health = (await response.json()) as Health;
		return health.database === 'ok'
			? { kind: 'ok', version: health.version }
			: { kind: 'database_unavailable', version: health.version };
	} catch {
		return { kind: 'unreachable' };
	}
}
