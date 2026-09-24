/**
 * Calls to the remotehub API. Errors arrive as problem responses with a code
 * (see errors.ts); network failures get the code `network`.
 */
import { readProblem } from './errors';

export type ApiResult<T> =
	| { ok: true; data: T }
	| { ok: false; status: number; code: string; params: Record<string, unknown> };

export const NETWORK_ERROR = 'network';

export async function api<T>(
	method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE',
	path: string,
	body?: unknown,
	fetcher: typeof fetch = fetch
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
