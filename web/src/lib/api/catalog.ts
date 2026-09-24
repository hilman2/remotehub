/** Folders, devices, credentials and grants (crates/server/src/api/catalog.rs). */
import { api } from './client';

export type Role = 'list' | 'connect' | 'reveal' | 'edit' | 'manage';
export const ROLES: readonly Role[] = ['list', 'connect', 'reveal', 'edit', 'manage'];

export type Protocol = 'ssh' | 'rdp' | 'vnc';
export const PROTOCOLS: readonly Protocol[] = ['ssh', 'rdp', 'vnc'];
export const DEFAULT_PORTS: Record<Protocol, number> = { ssh: 22, rdp: 3389, vnc: 5900 };

export type AuthMode = 'stored' | 'ask' | 'own';
export const AUTH_MODES: readonly AuthMode[] = ['stored', 'ask', 'own'];

export type ObjectKind = 'folder' | 'device' | 'credential';

export interface Folder {
	id: string;
	parent_id: string | null;
	name: string;
	/** null: only shown as the way to something visible inside */
	role: Role | null;
}

export interface Device {
	id: string;
	folder_id: string;
	name: string;
	protocol: Protocol;
	host: string;
	port: number;
	auth_mode: AuthMode;
	credential_id: string | null;
	description: string;
	/** SHA-256 fingerprint of the pinned host key (SSH), if pinned. */
	host_key_fingerprint: string | null;
	role: Role;
}

export type CredentialKind = 'password' | 'ssh_key';

export interface Credential {
	id: string;
	folder_id: string;
	name: string;
	kind: CredentialKind;
	username: string;
	domain: string;
	version: number;
	/** SSH keys only: what identifies the key, never the key itself. */
	key_algorithm: string | null;
	key_fingerprint: string | null;
	has_certificate: boolean;
	role: Role;
}

export interface Tree {
	folders: Folder[];
	devices: Device[];
	credentials: Credential[];
	may_create_top_level: boolean;
}

export interface DeviceInput {
	folder_id: string;
	name: string;
	protocol: Protocol;
	host: string;
	port: number;
	auth_mode: AuthMode;
	credential_id: string | null;
	description: string;
}

export interface CredentialInput {
	folder_id: string;
	name: string;
	username: string;
	domain: string;
	kind?: CredentialKind;
	/** Required when creating; omitted when updating keeps the password. */
	password?: string;
	/** SSH keys: required when creating; omitted when updating keeps key, passphrase and certificate. */
	private_key?: string;
	passphrase?: string;
	certificate?: string;
}

export interface GrantRow {
	id: string;
	folder_id: string | null;
	device_id: string | null;
	credential_id: string | null;
	principal_kind: 'user' | 'group';
	principal_sid: string;
	principal_name: string;
	role: Role;
}

export interface Principal {
	kind: 'user' | 'group';
	sid: string;
	name: string;
	detail: string | null;
}

/** Whether `role` includes `needed`. */
export function allows(role: Role | null, needed: Role): boolean {
	return role !== null && ROLES.indexOf(role) >= ROLES.indexOf(needed);
}

export const loadTree = () => api<Tree>('GET', '/api/tree');

export const createFolder = (parent_id: string | null, name: string) =>
	api<{ id: string }>('POST', '/api/folders', { parent_id, name });
export const renameFolder = (id: string, name: string) =>
	api('PATCH', `/api/folders/${id}`, { name });
export const deleteFolder = (id: string) => api('DELETE', `/api/folders/${id}`);

export const createDevice = (input: DeviceInput) =>
	api<{ id: string }>('POST', '/api/devices', input);
export const updateDevice = (id: string, input: DeviceInput) =>
	api('PUT', `/api/devices/${id}`, input);
export const deleteDevice = (id: string) => api('DELETE', `/api/devices/${id}`);
/** Forgets the pinned host key; the next connection pins the key presented then. */
export const resetHostKey = (id: string) => api('DELETE', `/api/devices/${id}/host-key`);

export const createCredential = (input: CredentialInput) =>
	api<{ id: string }>('POST', '/api/credentials', input);
export const updateCredential = (id: string, input: CredentialInput) =>
	api('PUT', `/api/credentials/${id}`, input);
export const deleteCredential = (id: string) => api('DELETE', `/api/credentials/${id}`);

export const loadGrants = (kind: ObjectKind, id: string) =>
	api<{ direct: GrantRow[]; inherited: GrantRow[] }>('GET', `/api/grants?kind=${kind}&id=${id}`);
export const addGrant = (kind: ObjectKind, id: string, principal: Principal, role: Role) =>
	api('POST', '/api/grants', {
		object: { kind, id },
		principal_kind: principal.kind,
		principal_sid: principal.sid,
		principal_name: principal.name,
		role
	});
export const removeGrant = (id: string) => api('DELETE', `/api/grants/${id}`);

export const searchPrincipals = (query: string) =>
	api<Principal[]>('GET', `/api/directory/principals?q=${encodeURIComponent(query)}`);
