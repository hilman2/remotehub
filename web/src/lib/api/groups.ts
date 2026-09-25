/** Groups of remotehub's own (crates/server/src/api/groups.rs, #105). */
import type { Principal } from './catalog';
import { api } from './client';

export interface GroupMember {
	/** Named like the principal of a grant. */
	sid: string;
	kind: 'user' | 'group';
	name: string;
}

export interface Group {
	id: string;
	name: string;
	description: string;
	members: GroupMember[];
}

export const loadGroups = () => api<Group[]>('GET', '/api/groups');
export const createGroup = (name: string, description: string) =>
	api<{ id: string }>('POST', '/api/groups', { name, description });
export const updateGroup = (id: string, name: string, description: string) =>
	api('PATCH', `/api/groups/${id}`, { name, description });
export const deleteGroup = (id: string) => api('DELETE', `/api/groups/${id}`);
export const addMember = (id: string, member: Principal) =>
	api('PUT', `/api/groups/${id}/members/${encodeURIComponent(member.sid)}`, {
		principal_kind: member.kind,
		principal_name: member.name
	});
export const removeMember = (id: string, sid: string) =>
	api('DELETE', `/api/groups/${id}/members/${encodeURIComponent(sid)}`);
