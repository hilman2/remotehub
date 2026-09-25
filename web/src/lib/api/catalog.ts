/** Folders, devices, credentials and grants (crates/server/src/api/catalog.rs). */
import { api, lasting } from './client';
import type { KeyboardLayout } from './generated/keyboard';

export { KEYBOARD_LAYOUTS, type KeyboardLayout } from './generated/keyboard';

export type Role = 'list' | 'connect' | 'reveal' | 'edit' | 'manage';
export const ROLES: readonly Role[] = ['list', 'connect', 'reveal', 'edit', 'manage'];

export type Protocol = 'ssh' | 'rdp' | 'vnc' | 'https';
export const PROTOCOLS: readonly Protocol[] = ['ssh', 'rdp', 'vnc', 'https'];
export const DEFAULT_PORTS: Record<Protocol, number> = {
	ssh: 22,
	rdp: 3389,
	vnc: 5900,
	https: 443
};

/** Protocols shown as a picture (through guacd) rather than a terminal. */
export const isGraphical = (protocol: Protocol) => protocol !== 'ssh';

export type AuthMode = 'stored' | 'device' | 'ask' | 'own' | 'certificate' | 'laps';
export const AUTH_MODES: readonly AuthMode[] = [
	'stored',
	'device',
	'ask',
	'own',
	'laps',
	'certificate'
];

/**
 * Sign-in modes a protocol offers: certificates are SSH's own, and LAPS
 * keeps the local administrator's password, which only SSH and RDP use.
 */
export function authModesFor(protocol: Protocol): readonly AuthMode[] {
	return AUTH_MODES.filter(
		(mode) =>
			(mode !== 'certificate' || protocol === 'ssh') &&
			(mode !== 'laps' || protocol === 'ssh' || protocol === 'rdp')
	);
}

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
	/** RDP only; null uses the instance's default. */
	keyboard_layout: KeyboardLayout | null;
	/** RDP and HTTPS: SHA-256 fingerprint of the pinned certificate, if pinned. */
	certificate_fingerprint: string | null;
	/** SHA-256 fingerprint of the pinned host key (SSH), if pinned. */
	host_key_fingerprint: string | null;
	/** The site connector the device is reached through; null: directly. */
	connector_id: string | null;
	/**
	 * Sign-in mode `device`: its own credentials as far as they are shown;
	 * password and key stay on the server.
	 */
	username: string;
	domain: string;
	secret_kind: CredentialKind;
	key_algorithm: string | null;
	key_fingerprint: string | null;
	has_certificate: boolean;
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
	url: string;
	notes: string;
	/** One of KeePass' standard icons (lib/vault/icons.ts). */
	icon: number;
	fields: CredentialField[];
	role: Role;
}

export interface Tree {
	folders: Folder[];
	devices: Device[];
	credentials: Credential[];
	may_create_top_level: boolean;
	/** Folders this user has open in the tree; all others are closed. */
	open: string[];
	/** Whether this user states a purpose before every connection (#90). */
	purpose_required: boolean;
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
	keyboard_layout: KeyboardLayout | null;
	connector_id: string | null;
	/** Sign-in mode `device` only. */
	username?: string;
	domain?: string;
	secret_kind?: CredentialKind;
	/** Left out on a change: the stored secret stays, unless the target or its kind changed. */
	password?: string;
	private_key?: string;
	passphrase?: string;
	certificate?: string;
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
	url?: string;
	notes?: string;
	icon?: number;
	/** A protected field without `value` keeps the one it has. */
	fields?: { name: string; protected?: boolean; value?: string }[];
}

/** A custom field of a credential (#98); protected ones come without value. */
export interface CredentialField {
	name: string;
	protected?: boolean;
	value?: string;
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
	/** RFC 3339, UTC: when a just-in-time grant runs out. */
	expires_at: string | null;
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
/** Opens or closes a folder in this user's tree; the server keeps it. */
export const setFolderOpen = (id: string, open: boolean) =>
	api('PUT', `/api/folders/${id}/open`, { open }, lasting);

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
