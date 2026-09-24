/** The signed-in user, shared by all pages. */
import { api, type ApiResult } from './api/client';

export interface User {
	username: string;
	display_name: string;
	kind: 'directory' | 'local';
}

export const session = $state<{ user: User | null; loaded: boolean }>({
	user: null,
	loaded: false
});

export async function loadSession(): Promise<void> {
	const result = await api<User>('GET', '/api/session');
	session.user = result.ok ? result.data : null;
	session.loaded = true;
}

export async function signIn(username: string, password: string): Promise<ApiResult<User>> {
	const result = await api<User>('POST', '/api/session', { username, password });
	if (result.ok) session.user = result.data;
	return result;
}

export async function signOut(): Promise<void> {
	await api('DELETE', '/api/session');
	session.user = null;
}
