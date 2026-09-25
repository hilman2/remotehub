/** The signed-in user, shared by all pages. */
import { api, type ApiResult } from './api/client';
import { loadSetupStatus, type SetupPhase } from './api/setup';
import { endSession, type Provider } from './kratos/flow';

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
 * Where the installation stands with its setup (#143); null until known.
 * The layout sends everyone to the wizard while it is `pending`, and the
 * administrator while it is `administrator`.
 */
export const setup = $state<{ phase: SetupPhase | null }>({ phase: null });

export async function loadSetup(): Promise<void> {
	const result = await loadSetupStatus();
	// Without an answer, nothing is held back: the pages say what fails.
	setup.phase = result.ok ? result.data.phase : 'complete';
}

/**
 * The second step of a directory sign-in: a code of the app, the key the
 * server offered when the app is set up now (#107), or a security key's
 * answer to the server's challenge (#129).
 */
export interface DirectoryFactor {
	code?: string;
	totp_secret?: string;
	security_key?: { challenge_id: string; credential: unknown };
}

/**
 * Signs in with the directory. `second` carries the second step once the
 * server asked for it.
 */
export async function signIn(
	username: string,
	password: string,
	second: DirectoryFactor = {}
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
	/** OpenID Connect providers for local accounts (#109). */
	providers: Provider[];
	/** A mail server is set: a forgotten password's code goes out by mail (#145). */
	mail: boolean;
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
