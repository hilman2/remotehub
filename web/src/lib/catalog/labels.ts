/** Labels for roles, protocols and sign-in modes; `Record` makes a missing one a type error. */
import type { AuthMode, ObjectKind, Protocol, Role } from '$lib/api/catalog';
import { m } from '$lib/paraglide/messages';

export const ROLE_LABELS: Record<Role, () => string> = {
	list: m.role_list,
	connect: m.role_connect,
	reveal: m.role_reveal,
	edit: m.role_edit,
	manage: m.role_manage
};

export const PROTOCOL_LABELS: Record<Protocol, () => string> = {
	ssh: m.protocol_ssh,
	rdp: m.protocol_rdp,
	vnc: m.protocol_vnc
};

export const AUTH_MODE_LABELS: Record<AuthMode, () => string> = {
	stored: m.auth_stored,
	ask: m.auth_ask,
	own: m.auth_own
};

export const KIND_LABELS: Record<ObjectKind, () => string> = {
	folder: m.catalog_kind_folder,
	device: m.catalog_kind_device,
	credential: m.catalog_kind_credential
};
