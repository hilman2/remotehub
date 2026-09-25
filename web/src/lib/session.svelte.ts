/** The signed-in user, shared by all pages. */
import { api, type ApiResult } from './api/client';
import { endSession } from './kratos/flow';

export interface User {
	username: string;
	display_name: string;
	/** From AD, a break-glass account, or a local account in Kratos (#103). */
	kind: 'directory' | 'break_glass' | 'local';
	/** May manage remotehub: folders at the top, grants, the audit log. */
	admin: boolean;
	/** Roles for remotehub itself (#106). */
	roles: Role[];
}

export type Role = 'administrator' | 'auditor' | 'security_officer';

/** Reads the audit log: auditors and administrators. */
export const isAuditor = (user: User | null) =>
	!!user && (user.admin || user.roles.includes('auditor'));

/** Approves the recovery of personal vaults (#95). */
export const isSecurityOfficer = (user: User | null) =>
	!!user && user.roles.includes('security_officer');

export const session = $state<{ user: User | null; loaded: boolean }>({
	user: null,
	loaded: false
});

export async function loadSession(): Promise<void> {
	const result = await api<User>('GET', '/api/session');
	session.user = result.ok ? result.data : null;
	session.loaded = true;
}

/**
 * Signs in with the directory. `second` carries a code of the authenticator
 * app once the server asked for one, and the key it offered when the app is
 * set up now (#107).
 */
export async function signIn(
	username: string,
	password: string,
	second: { code?: string; totp_secret?: string } = {}
): Promise<ApiResult<User>> {
	const result = await api<User>('POST', '/api/session', { username, password, ...second });
	if (result.ok) session.user = result.data;
	return result;
}

export async function signOut(): Promise<void> {
	const local = session.user?.kind === 'local';
	await api('DELETE', '/api/session');
	// The server ends the Kratos session too; this clears its cookie.
	if (local) await endSession();
	session.user = null;
}

/** Which ways to sign in this instance offers. */
export interface Methods {
	/** Active Directory. */
	directory: boolean;
	/** Local accounts in Kratos (#103). */
	local: boolean;
}

export const loadMethods = () => api<Methods>('GET', '/api/session/methods');

/**
 * Turns this browser's Kratos session into a remotehub session. Fails with
 * `second_factor_required` or `second_factor_setup_required` until the
 * account's second factor was used.
 */
export async function signInLocal(): Promise<ApiResult<User>> {
	const result = await api<User>('POST', '/api/session/local');
	if (result.ok) session.user = result.data;
	return result;
}

export async function signInBreakGlass(
	username: string,
	password: string,
	code: string
): Promise<ApiResult<User>> {
	const result = await api<User>('POST', '/api/session/break-glass', { username, password, code });
	if (result.ok) session.user = result.data;
	return result;
}
