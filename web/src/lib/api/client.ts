/**
 * Calls to the remotehub API. Errors arrive as problem responses with a code
 * (see errors.ts); network failures get the code `network`.
 */
import { readProblem } from './errors';

export type ApiResult<T> =
	| { ok: true; data: T }
	| { ok: false; status: number; code: string; params: Record<string, unknown> };

export const NETWORK_ERROR = 'network';

/**
 * A fetch that finishes even when the page goes right after: for small
 * writes of preferences that a reload or a closed tab must not lose. Pass it
 * as `fetcher` to `api`.
 */
export const lasting: typeof fetch = (input, init) => fetch(input, { ...init, keepalive: true });

/**
 * A fetch for what the page asks on its own, e.g. a state read every
 * minute: it does not keep the session alive, or the session would never
 * lock (#241). Pass it as `fetcher` to `api`.
 */
export const background: typeof fetch = (input, init) =>
	fetch(input, {
		...init,
		headers: { ...(init?.headers as Record<string, string>), 'x-remotehub-background': '1' }
	});

/**
 * What the page does when the server refuses a request for the session's
 * sake; the layout sets it. `locked` and `confirm` resolve to true once the
 * user confirmed with the second factor, and the request is sent again.
 */
export const guard: {
	/** The session is locked (#241). */
	locked?: () => Promise<boolean>;
	/** The request needs a confirmation of the last minute (#242). */
	confirm?: () => Promise<boolean>;
	/** The session is over: expired or ended elsewhere (#240). */
	ended?: () => void;
} = {};

export async function api<T>(
	method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE',
	path: string,
	body?: unknown,
	fetcher: typeof fetch = fetch
): Promise<ApiResult<T>> {
	const result = await send<T>(method, path, body, fetcher);
	if (result.ok) return result;
	const again =
		(result.code === 'session_locked' && guard.locked) ||
		(result.code === 'confirmation_required' && guard.confirm);
	if (again && (await again())) return api(method, path, body, fetcher);
	if (result.code === 'unauthenticated') guard.ended?.();
	return result;
}

async function send<T>(
	method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE',
	path: string,
	body: unknown,
	fetcher: typeof fetch
): Promise<ApiResult<T>> {
	const headers: Record<string, string> = { accept: 'application/json' };
	if (body !== undefined) headers['content-type'] = 'application/json';
	let response: Response;
	try {
		response = await fetcher(path, {
			method,
			headers,
			credentials: 'same-origin',
			body: body === undefined ? undefined : JSON.stringify(body)
		});
	} catch {
		return { ok: false, status: 0, code: NETWORK_ERROR, params: {} };
	}
	if (response.ok) {
		const data = response.status === 204 ? undefined : await response.json();
		return { ok: true, data: data as T };
	}
	const problem = await readProblem(response);
	return {
		ok: false,
		status: response.status,
		code: problem?.code ?? 'internal',
		params: problem?.params ?? {}
	};
}
