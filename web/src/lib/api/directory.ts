/** The directory connection (crates/server/src/api/directory.rs, #144). */
import { api } from './client';
import { assignRole, loadRoles, revokeRole } from './roles';
import type { Principal } from './catalog';

/** Everything about the connection but the password. */
export interface Connection {
	url: string;
	starttls: boolean;
	ca_pem: string | null;
	bind_dn: string;
	base_dn: string;
	user_filter: string | null;
	timeout_seconds: number;
}

/** What the form sends: no password keeps the stored one. */
export interface ConnectionInput extends Connection {
	password: string | null;
}

export type CheckStep = 'resolve' | 'connect' | 'tls' | 'bind' | 'search';
export type CheckReason =
	| 'not_found'
	| 'refused'
	| 'timeout'
	| 'start_tls_refused'
	| 'unknown_ca'
	| 'name_mismatch'
	| 'expired'
	| 'invalid_credentials'
	| 'no_such_base'
	| 'other';

/** The topmost certificate the server presented, to trust after comparing it. */
export interface PresentedCa {
	/** A CA's certificate; otherwise the server's own. */
	authority: boolean;
	subject: string;
	fingerprint: string;
	pem: string;
}

export interface CheckFailure {
	step: CheckStep;
	reason: CheckReason;
	/** The server's or the library's own words. */
	detail: string;
	ca?: PresentedCa;
}

export interface Found {
	users: number;
	more: boolean;
	sample: string[];
}

export const loadConnection = () =>
	api<{ connection: Connection | null }>('GET', '/api/settings/directory');
export const checkConnection = (input: ConnectionInput) =>
	api<{ found?: Found; failure?: CheckFailure }>('POST', '/api/settings/directory/check', input);
export const saveConnection = (input: ConnectionInput) =>
	api<Found>('PUT', '/api/settings/directory', input);
export const removeConnection = () => api('DELETE', '/api/settings/directory');

/** The administrators, as the list of a PrincipalRules. */
export async function loadAdministrators() {
	const result = await loadRoles();
	if (!result.ok) return result;
	const members = result.data.find((roles) => roles.role === 'administrator')?.members ?? [];
	return {
		ok: true as const,
		data: members.map((member) => ({
			principal_sid: member.sid,
			principal_kind: member.kind,
			principal_name: member.name
		}))
	};
}
export const addAdministrator = (principal: Principal) => assignRole('administrator', principal);
export const removeAdministrator = (sid: string) => revokeRole('administrator', sid);
