/** Site connectors (crates/server/src/api/connectors.rs, ADR 0008). */
import { api } from './client';

export interface Connector {
	id: string;
	name: string;
	/** Whether its control connection to remotehub is open now. */
	online: boolean;
	/** Connections it carries now. */
	streams: number;
	/** Connections it has carried since remotehub started. */
	streams_carried: number;
	/**
	 * Whether the customer lets remotehub in (#165): the whole network, single
	 * devices (`partly`, #180), or nothing; null without a recent report.
	 */
	access: 'open' | 'partly' | 'closed' | null;
	/** RFC 3339, while open until a point in time. */
	open_until: string | null;
	/** The groups and devices of the customer's list that are open (#180). */
	open_groups: OpenGroup[];
	open_devices: OpenDevice[];
	/** RFC 3339, UTC; null before its first connection. */
	last_seen_at: string | null;
}

export interface OpenGroup {
	name: string;
	/** RFC 3339; null while open without end. */
	until: string | null;
}

export interface OpenDevice {
	name: string;
	/** As the customer wrote it: an address, a range or a host name. */
	address: string;
	/** Such as `22,8000-8100`. */
	ports: string;
	/** RFC 3339; null while open without end. */
	until: string | null;
}

/**
 * Whether the customer lets remotehub reach one device now (#180), as its
 * connector says.
 */
export interface ConnectorAccess {
	state: 'direct' | 'open' | 'closed' | 'offline' | 'unknown';
	/** RFC 3339, while open until a point in time. */
	until: string | null;
}

/** A new connector; the token is shown this once and kept nowhere. */
export interface CreatedConnector {
	id: string;
	name: string;
	token: string;
}

export const loadConnectors = () => api<Connector[]>('GET', '/api/connectors');
export const createConnector = (name: string) =>
	api<CreatedConnector>('POST', '/api/connectors', { name });
export const deleteConnector = (id: string) => api('DELETE', `/api/connectors/${id}`);
export const loadConnectorAccess = (deviceId: string) =>
	api<ConnectorAccess>('GET', `/api/devices/${deviceId}/connector-access`);
