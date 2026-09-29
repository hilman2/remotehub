/**
 * Folders, devices, login profiles, credentials and grants
 * (crates/server/src/api/catalog.rs, profiles.rs).
 */
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

export type AuthMode = 'profile' | 'device' | 'ask' | 'own' | 'certificate' | 'laps';
export const AUTH_MODES: readonly AuthMode[] = [
	'profile',
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

export type ObjectKind = 'folder' | 'device' | 'credential' | 'collection' | 'profile';

export interface Folder {
	id: string;
	parent_id: string | null;
	name: string;
	/** null: only shown as the way to something visible inside */
	role: Role | null;
	/** The site connector it names for what is in it (#176); null: its parent's. */
	connector_id: string | null;
}

/** How a device is reached (#176): its folder's connector, none, or its own. */
export type ConnectorMode = 'inherit' | 'direct' | 'connector';

export interface Device {
	id: string;
	folder_id: string;
	name: string;
	protocol: Protocol;
	host: string;
	port: number;
	auth_mode: AuthMode;
	/** Sign-in mode `profile`: the login profile it uses (#192). */
	profile_id: string | null;
	description: string;
	/** Words everyone who sees it may change, for everyone's search (#215). */
	keywords: string;
	/** Who changed them last, by name, and when (ISO 8601); null if nobody has. */
	keywords_changed_by: string | null;
	keywords_changed_at: string | null;
	/** RDP only; null uses the instance's default. */
	keyboard_layout: KeyboardLayout | null;
	/** RDP and HTTPS: SHA-256 fingerprint of the pinned certificate, if pinned. */
	certificate_fingerprint: string | null;
	/** SHA-256 fingerprint of the pinned host key (SSH), if pinned. */
	host_key_fingerprint: string | null;
	connector_mode: ConnectorMode;
	/** Its own connector, with `connector_mode` `connector`. */
	connector_id: string | null;
	/** The connector it is reached through, however chosen; null: directly. */
	reached_through: string | null;
	/** Asks for a confirmation with the second factor before every connection (#243). */
	requires_confirmation: boolean;
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

/**
 * A login many devices share (#192), a password or an SSH key. It lies in a
 * device folder and has that folder's grants; one at the top level
 * (`folder_id` null) is for administrators. Devices in its folder and below
 * may use it.
 */
export interface Profile {
	id: string;
	folder_id: string | null;
	name: string;
	username: string;
	domain: string;
	secret_kind: CredentialKind;
	/** SSH keys only: what identifies the key, never the key itself. */
	key_algorithm: string | null;
	key_fingerprint: string | null;
	has_certificate: boolean;
	/** RFC 3339, UTC. */
	updated_at: string;
	role: Role;
}

export interface ProfileInput {
	folder_id: string | null;
	name: string;
	username: string;
	domain: string;
	secret_kind: CredentialKind;
	/** Required when creating or changing the kind; omitted keeps what is stored. */
	password?: string;
	private_key?: string;
	passphrase?: string;
	certificate?: string;
}

/** Where shared credentials live, apart from the device folders (#190). */
export interface Collection {
	id: string;
	parent_id: string | null;
	name: string;
	/** null: only shown as the way to something visible inside */
	role: Role | null;
}

export interface Credential {
	id: string;
	/** The collection it lives in (#190). */
	collection_id: string;
	name: string;
	/** "DOMAIN\user" where a domain belongs, as KeePass keeps it. */
	username: string;
	version: number;
	url: string;
	notes: string;
	/** One of KeePass' standard icons (lib/vault/icons.ts). */
	icon: number;
	fields: CredentialField[];
	/** Files kept with it (#100), without content. */
	attachments: { id: string; name: string; size: number }[];
	/** Words to find it by (#193). */
	tags: string[];
	/** `YYYY-MM-DD`: when its password runs out; null: never. */
	expires_on: string | null;
	/** Whether it has a one-time password; codes come from the server. */
	has_totp: boolean;
	/** RFC 3339, UTC. */
	updated_at: string;
	/** RFC 3339, UTC: in the recycle bin since; null: in its collection. */
	deleted_at: string | null;
	role: Role;
}

export interface Tree {
	folders: Folder[];
	devices: Device[];
	profiles: Profile[];
	collections: Collection[];
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
	profile_id: string | null;
	description: string;
	keyboard_layout: KeyboardLayout | null;
	connector_mode: ConnectorMode;
	/** Set exactly with `connector_mode` `connector`. */
	connector_id: string | null;
	requires_confirmation: boolean;
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
	collection_id: string;
	name: string;
	username: string;
	/** Required when creating; omitted when updating keeps the password. */
	password?: string;
	url?: string;
	notes?: string;
	icon?: number;
	/** A protected field without `value` keeps the one it has. */
	fields?: { name: string; protected?: boolean; value?: string }[];
	tags?: string[];
	expires_on?: string | null;
	/** An `otpauth://` link or base32 secret; omitted keeps it, empty removes it. */
	totp?: string;
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

/** `connector_id`: the site connector for what is in the folder; null: its parent's. */
export const createFolder = (parent_id: string | null, name: string, connector_id: string | null) =>
	api<{ id: string }>('POST', '/api/folders', { parent_id, name, connector_id });
export const updateFolder = (id: string, name: string, connector_id: string | null) =>
	api('PATCH', `/api/folders/${id}`, { name, connector_id });
/** Into another folder, or to the top level with null (#214). */
export const moveFolder = (id: string, parent_id: string | null) =>
	api('PATCH', `/api/folders/${id}`, { parent_id });
export const deleteFolder = (id: string) => api('DELETE', `/api/folders/${id}`);

/** Collections of shared credentials (#190); `parent_id` null: at the top. */
export const createCollection = (parent_id: string | null, name: string) =>
	api<{ id: string }>('POST', '/api/collections', { parent_id, name });
export const updateCollection = (
	id: string,
	change: { name?: string; parent_id?: string | null }
) => api('PATCH', `/api/collections/${id}`, change);
export const deleteCollection = (id: string) => api('DELETE', `/api/collections/${id}`);

export const createDevice = (input: DeviceInput) =>
	api<{ id: string }>('POST', '/api/devices', input);
export const updateDevice = (id: string, input: DeviceInput) =>
	api('PUT', `/api/devices/${id}`, input);
export const deleteDevice = (id: string) => api('DELETE', `/api/devices/${id}`);
/** Into another folder; nothing else of the device changes (#214). */
export const moveDevice = (id: string, folder_id: string) =>
	api('PUT', `/api/devices/${id}/folder`, { folder_id });
/** Forgets the pinned host key; the next connection pins the key presented then. */
export const resetHostKey = (id: string) => api('DELETE', `/api/devices/${id}/host-key`);
/** Sets the search words of a device (#215); whoever sees it may. */
export const setKeywords = (id: string, keywords: string) =>
	api('PUT', `/api/devices/${id}/keywords`, { keywords });
/** Opens or closes a folder in this user's tree; the server keeps it. */
export const setFolderOpen = (id: string, open: boolean) =>
	api('PUT', `/api/folders/${id}/open`, { open }, lasting);

export const createProfile = (input: ProfileInput) =>
	api<{ id: string }>('POST', '/api/profiles', input);
export const updateProfile = (id: string, input: ProfileInput) =>
	api('PUT', `/api/profiles/${id}`, input);
export const deleteProfile = (id: string) => api('DELETE', `/api/profiles/${id}`);

export const createCredential = (input: CredentialInput) =>
	api<{ id: string }>('POST', '/api/credentials', input);
export const updateCredential = (id: string, input: CredentialInput) =>
	api('PUT', `/api/credentials/${id}`, input);
/** Into the recycle bin (#193); for one already there, for good. */
/** Moves a credential into the recycle bin; with `purge`, deletes it for good. */
export const deleteCredential = (id: string, purge = false) =>
	api('DELETE', `/api/credentials/${id}${purge ? '?purge=true' : ''}`);
/** Out of the recycle bin, back into its collection. */
export const restoreCredential = (id: string) => api('POST', `/api/credentials/${id}/restore`);

export const loadGrants = (kind: ObjectKind, id: string) =>
	api<{ direct: GrantRow[]; inherited: GrantRow[] }>('GET', `/api/grants?kind=${kind}&id=${id}`);
/**
 * Grants `role`; `expiresAt` (RFC 3339) makes a grant of its own that ends
 * by itself (#178), beside one without end.
 */
export const addGrant = (
	kind: ObjectKind,
	id: string,
	principal: Principal,
	role: Role,
	expiresAt: string | null = null
) =>
	api('POST', '/api/grants', {
		object: { kind, id },
		principal_kind: principal.kind,
		principal_sid: principal.sid,
		principal_name: principal.name,
		role,
		expires_at: expiresAt
	});
export const removeGrant = (id: string) => api('DELETE', `/api/grants/${id}`);

export const searchPrincipals = (query: string) =>
	api<Principal[]>('GET', `/api/directory/principals?q=${encodeURIComponent(query)}`);
