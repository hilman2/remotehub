/** The Access page (crates/server/src/api/access.rs, #178). */
import type { Role as SiteRole } from '$lib/session.svelte';
import type { ObjectKind, Role } from './catalog';
import { api } from './client';
import type { UserRow } from './users';

/** A user or group as grants and memberships name it. */
export interface Named {
	sid: string;
	name: string;
}

export interface UserGroup extends Named {
	source: 'directory' | 'own';
	/** For an own group: the directory group through which the user is in it. */
	via: Named | null;
}

export interface AccessUser extends UserRow {
	/** Named like the principal of a grant; null for a break-glass account. */
	principal: string | null;
	/** Directory groups of the latest sign-in, then own groups. */
	groups: UserGroup[];
	/** Roles for remotehub itself, and whom they are assigned to. */
	roles: { role: SiteRole; via: Named }[];
}

export interface GroupMember extends Named {
	kind: 'user' | 'group';
	/** The remotehub user, if the member is one. */
	user_id: string | null;
}

export interface AccessGroup extends Named {
	source: 'directory' | 'own';
	/** An own group's id. */
	id: string | null;
	description: string;
	/** An own group's members; a directory group's users as of their latest sign-in. */
	members: GroupMember[];
	/** The own groups it is in. */
	member_of: Named[];
	roles: SiteRole[];
	/** Grants to it that have not ended. */
	grants: number;
}

export interface Overview {
	users: AccessUser[];
	groups: AccessGroup[];
}

export interface PrincipalGrant {
	id: string;
	object: { kind: ObjectKind; id: string };
	name: string;
	/** The folders above it, from the top. */
	path: string[];
	role: Role;
	/** RFC 3339, when it ends by itself. */
	expires_at: string | null;
}

export const loadAccess = () => api<Overview>('GET', '/api/access');
export const loadPrincipalGrants = (sid: string) =>
	api<PrincipalGrant[]>('GET', `/api/access/grants?principal=${encodeURIComponent(sid)}`);
