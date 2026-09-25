/** User management for administrators (crates/server/src/api/users.rs, #104). */
import { api } from './client';
import type { SendFailure } from './mail';

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
	/** Signs in with a second factor (#107). */
	second_factor: boolean;
	/** Sessions that have not run out. */
	sessions: number;
}

/** A one-time code for a local account; shown once and kept nowhere. */
export interface OneTimeCode {
	link: string;
	code: string;
	/** RFC 3339. */
	expires_at: string;
	/** It went out by mail as well (#145). */
	mailed: boolean;
	mail_failure?: SendFailure;
}

/** Whether and in which language a code goes out by mail (#145). */
export interface MailRequest {
	send_mail: boolean;
	language: string;
}

export const loadUsers = () => api<UserRow[]>('GET', '/api/users');
export const inviteUser = (email: string, name: string, mail: MailRequest) =>
	api<OneTimeCode>('POST', '/api/users/invite', { email, name, ...mail });
export const blockUser = (id: string) => api('POST', `/api/users/${id}/block`);
export const unblockUser = (id: string) => api('DELETE', `/api/users/${id}/block`);
export const endSessions = (id: string) => api('DELETE', `/api/users/${id}/sessions`);
export const issueRecovery = (id: string, mail: MailRequest) =>
	api<OneTimeCode>('POST', `/api/users/${id}/recovery`, mail);
export const deleteUser = (id: string) => api('DELETE', `/api/users/${id}`);
