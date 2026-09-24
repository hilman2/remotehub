/** Just-in-time access (crates/server/src/api/requests.rs). */
import { allows, type ObjectKind, type Role } from './catalog';
import { api } from './client';

export type RequestStatus = 'pending' | 'approved' | 'denied' | 'cancelled';

export interface AccessRequest {
	id: string;
	object: { kind: ObjectKind; id: string };
	object_name: string;
	requester_name: string;
	role: Role;
	minutes: number;
	reason: string;
	status: RequestStatus;
	decider_name: string | null;
	/** RFC 3339, UTC */
	created_at: string;
	decided_at: string | null;
	/** When an approved request's grant runs out. */
	expires_at: string | null;
}

export interface Requests {
	mine: AccessRequest[];
	to_decide: AccessRequest[];
}

/** How long access may be asked for, in minutes, as the form offers it. */
export const DURATIONS: readonly number[] = [60, 240, 480, 1440];

/** The roles someone holding `held` may ask for: only connect and reveal, and only more than they have. */
export function requestableRoles(held: Role | null): Role[] {
	if (held === null) return [];
	return (['connect', 'reveal'] as const).filter((role) => !allows(held, role));
}

export const loadRequests = () => api<Requests>('GET', '/api/access-requests');
export const createRequest = (
	object: { kind: ObjectKind; id: string },
	role: Role,
	minutes: number,
	reason: string
) => api<{ id: string }>('POST', '/api/access-requests', { object, role, minutes, reason });
export const approveRequest = (id: string) => api('POST', `/api/access-requests/${id}/approve`);
export const denyRequest = (id: string) => api('POST', `/api/access-requests/${id}/deny`);
export const cancelRequest = (id: string) => api('DELETE', `/api/access-requests/${id}`);
