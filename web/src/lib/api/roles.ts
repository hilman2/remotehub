/** Roles for remotehub itself (crates/server/src/api/roles.rs, #106). */
import type { Role } from '$lib/session.svelte';
import type { Principal } from './catalog';
import { api } from './client';
import type { GroupMember } from './groups';

export interface RoleAssignments {
	role: Role;
	members: GroupMember[];
}

export const loadRoles = () => api<RoleAssignments[]>('GET', '/api/roles');
export const assignRole = (role: Role, member: Principal) =>
	api('PUT', `/api/roles/${role}/members/${encodeURIComponent(member.sid)}`, {
		principal_kind: member.kind,
		principal_name: member.name
	});
export const revokeRole = (role: Role, sid: string) =>
	api('DELETE', `/api/roles/${role}/members/${encodeURIComponent(sid)}`);
