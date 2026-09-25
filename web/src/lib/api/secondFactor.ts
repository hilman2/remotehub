/** The second factor of directory users (crates/server/src/api/second_factor.rs, #107). */
import type { Principal } from './catalog';
import { api } from './client';

export interface FactorStatus {
	/** Only directory users set one up in remotehub. */
	available: boolean;
	/** An authenticator app or a security key. */
	enrolled: boolean;
	/** A rule asks for one: the last one cannot be removed. */
	required: boolean;
	/** An authenticator app. */
	app: boolean;
	/** Security keys and passkeys (#129). */
	keys: SecurityKey[];
}

export interface SecurityKey {
	id: string;
	name: string;
	/** RFC 3339, UTC */
	created_at: string;
	last_used_at: string | null;
}

/** A challenge for a new key, and the options for `navigator.credentials.create()`. */
export interface KeyOffer {
	challenge_id: string;
	options: unknown;
}

/** A new authenticator app's key, and the URI the app reads. */
export interface FactorOffer {
	secret: string;
	uri: string;
}

export interface FactorRule {
	principal_sid: string;
	principal_kind: 'user' | 'group';
	principal_name: string;
}

export const loadFactor = () => api<FactorStatus>('GET', '/api/account/second-factor');
export const offerFactor = () => api<FactorOffer>('POST', '/api/account/second-factor/offer');
export const enrollFactor = (secret: string, code: string) =>
	api('PUT', '/api/account/second-factor', { secret, code });
export const removeFactor = (code: string) => api('DELETE', '/api/account/second-factor', { code });
export const resetFactor = (userId: string) => api('DELETE', `/api/users/${userId}/second-factor`);
export const offerKey = () => api<KeyOffer>('POST', '/api/account/security-keys/offer');
export const addKey = (challenge_id: string, name: string, credential: unknown) =>
	api('POST', '/api/account/security-keys', { challenge_id, name, credential });
export const removeKey = (id: string) => api('DELETE', `/api/account/security-keys/${id}`);

export const loadRules = () => api<FactorRule[]>('GET', '/api/second-factor-principals');
export const requireFactor = (principal: Principal) =>
	api('PUT', `/api/second-factor-principals/${encodeURIComponent(principal.sid)}`, {
		principal_kind: principal.kind,
		principal_name: principal.name
	});
export const waiveFactor = (sid: string) =>
	api('DELETE', `/api/second-factor-principals/${encodeURIComponent(sid)}`);
