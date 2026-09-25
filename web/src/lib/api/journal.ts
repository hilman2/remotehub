/**
 * A device's journal and who states a purpose before connecting (#90;
 * crates/server/src/api/journal.rs).
 */
import { api } from './client';
import type { Principal } from './catalog';

export interface JournalEntry {
	id: string;
	kind: 'connection' | 'note';
	/** The purpose of a connection (may be empty), or the note. */
	text: string;
	protocol: string | null;
	username: string;
	display_name: string;
	created_at: string;
	/** When the session ended; null while it runs or if the server stopped. */
	ended_at: string | null;
}

export interface PurposePrincipal {
	principal_sid: string;
	principal_kind: 'user' | 'group';
	principal_name: string;
}

const journalPath = (deviceId: string) => `/api/devices/${encodeURIComponent(deviceId)}/journal`;

export const loadJournal = (deviceId: string) => api<JournalEntry[]>('GET', journalPath(deviceId));

export const addNote = (deviceId: string, text: string) =>
	api('POST', journalPath(deviceId), { text });

export const loadPurposePrincipals = () =>
	api<PurposePrincipal[]>('GET', '/api/purpose-principals');

export const requirePurpose = (principal: Principal) =>
	api('PUT', `/api/purpose-principals/${encodeURIComponent(principal.sid)}`, {
		principal_kind: principal.kind,
		principal_name: principal.name
	});

export const waivePurpose = (sid: string) =>
	api('DELETE', `/api/purpose-principals/${encodeURIComponent(sid)}`);
