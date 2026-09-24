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
	/** RFC 3339, UTC; null before its first connection. */
	last_seen_at: string | null;
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
