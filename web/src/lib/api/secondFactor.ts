/** The second factor of directory users (crates/server/src/api/second_factor.rs, #107). */
import type { Principal } from './catalog';
import { api } from './client';

export interface FactorStatus {
	/** Only directory users set one up in remotehub. */
	available: boolean;
	enrolled: boolean;
	/** A rule asks for one: it cannot be removed. */
	required: boolean;
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

export const loadRules = () => api<FactorRule[]>('GET', '/api/second-factor-principals');
export const requireFactor = (principal: Principal) =>
	api('PUT', `/api/second-factor-principals/${encodeURIComponent(principal.sid)}`, {
		principal_kind: principal.kind,
		principal_name: principal.name
	});
export const waiveFactor = (sid: string) =>
	api('DELETE', `/api/second-factor-principals/${encodeURIComponent(sid)}`);
