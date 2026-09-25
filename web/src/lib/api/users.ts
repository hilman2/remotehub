/** User management for administrators (crates/server/src/api/users.rs, #104). */
import { api } from './client';

export type UserKind = 'directory' | 'local' | 'break_glass';

export interface UserRow {
	id: string;
	kind: UserKind;
	/** The e-mail address of a local account. */
	username: string;
	display_name: string;
	email: string | null;
	/** RFC 3339, UTC; null before the first sign-in. */
	last_sign_in_at: string | null;
	blocked: boolean;
	/** Sessions that have not run out. */
	sessions: number;
}

/** A one-time code for a local account; shown once and kept nowhere. */
export interface OneTimeCode {
	link: string;
	code: string;
	/** RFC 3339. */
	expires_at: string;
}

export const loadUsers = () => api<UserRow[]>('GET', '/api/users');
export const inviteUser = (email: string, name: string) =>
	api<OneTimeCode>('POST', '/api/users/invite', { email, name });
export const blockUser = (id: string) => api('POST', `/api/users/${id}/block`);
export const unblockUser = (id: string) => api('DELETE', `/api/users/${id}/block`);
export const endSessions = (id: string) => api('DELETE', `/api/users/${id}/sessions`);
export const issueRecovery = (id: string) => api<OneTimeCode>('POST', `/api/users/${id}/recovery`);
export const deleteUser = (id: string) => api('DELETE', `/api/users/${id}`);
