/** Asking the customer for access at a site connector (crates/server/src/api/connector_requests.rs, #181). */
import type { ObjectKind } from './catalog';
import { api } from './client';

export type ConnectorRequestStatus = 'pending' | 'approved' | 'refused' | 'cancelled' | 'expired';

export interface ConnectorRequest {
	id: string;
	connector_id: string;
	connector_name: string;
	/** The folder or device asked for; null once it is deleted. */
	object: { kind: ObjectKind; id: string } | null;
	object_name: string;
	/** What an approval opens. */
	targets: { name: string; host: string; port: number }[];
	requester_name: string;
	minutes: number;
	reason: string;
	status: ConnectorRequestStatus;
	/** The connector user who answered. */
	answered_by: string | null;
	answered_at: string | null;
	/** RFC 3339, when an approval ends. */
	until: string | null;
	created_at: string;
}

/** The caller's own requests, newest first; an administrator's hold everyone's. */
export const loadConnectorRequests = () =>
	api<ConnectorRequest[]>('GET', '/api/connector-requests');
export const askCustomer = (
	object: { kind: ObjectKind; id: string },
	minutes: number,
	reason: string
) => api<{ id: string }>('POST', '/api/connector-requests', { object, minutes, reason });
export const withdrawConnectorRequest = (id: string) =>
	api('DELETE', `/api/connector-requests/${id}`);
