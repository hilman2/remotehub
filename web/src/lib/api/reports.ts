/** Permission reports for auditors and administrators (crates/server/src/api/reports.rs, #110). */
import type { ObjectKind, Role } from './catalog';
import { api } from './client';

export interface Person {
	id: string;
	username: string;
	display_name: string;
	kind: 'directory' | 'local' | 'break_glass';
}

export interface FolderChoice {
	id: string;
	/** Folder names from the top. */
	path: string[];
}

export interface Reason {
	principal_sid: string;
	principal_name: string;
	role: Role;
	on: { kind: ObjectKind; id: string };
	on_name: string;
	/** From a folder above the object. */
	inherited: boolean;
}

export interface Reach {
	object: { kind: ObjectKind; id: string };
	name: string;
	path: string[];
	role: Role;
	/** Empty for an administrator. */
	reasons: Reason[];
}

export interface UserReport {
	user: Person;
	administrator: boolean;
	/** Where the directory groups came from. */
	groups_from: 'directory' | 'last_sign_in' | 'none';
	/** `[sid, name]` of everything counted as the user's. */
	principals: [string, string][];
	reach: Reach[];
}

export interface Holder {
	principal_sid: string;
	principal_name: string;
	role: Role;
	on: string;
	on_path: string[];
	inherited: boolean;
	/** Members of a group of remotehub's own. */
	members: string[];
}

export interface FolderReport {
	folder: FolderChoice;
	holders: Holder[];
}

export const loadPeople = () => api<Person[]>('GET', '/api/reports/people');
export const loadFolders = () => api<FolderChoice[]>('GET', '/api/reports/folders');
export const userReport = (id: string) => api<UserReport>('GET', `/api/reports/users/${id}`);
export const folderReport = (id: string) => api<FolderReport>('GET', `/api/reports/folders/${id}`);
