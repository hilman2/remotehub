/** The certificate of Caddy of the ops package (crates/server/src/api/certificate.rs, #146). */
import { api } from './client';

/** Where the certificate Caddy serves comes from. */
export type Source = 'lets_encrypt' | 'remotehub' | 'own' | 'other';

export interface CertificateInfo {
	subject: string;
	issuer: string;
	/** The DNS names it covers. */
	names: string[];
	/** Seconds since 1970. */
	not_before: number;
	not_after: number;
	/** SHA-256 in hex pairs. */
	fingerprint: string;
}

export interface OwnCertificate {
	info: CertificateInfo;
	/** It comes with the certificate that signed it, or signed itself. */
	chain_complete: boolean;
}

export interface CertificateStatus {
	/** The host a certificate has to cover. */
	host: string;
	/** Caddy of the package runs; otherwise a proxy of your own holds the certificate. */
	runs: boolean;
	served?: CertificateInfo;
	source?: Source;
	own?: OwnCertificate;
	/** Of the root certificate of Caddy's own CA, as /ca.crt hands it out. */
	root_fingerprint?: string;
}

/** Why the server refused a certificate (`params.reason` of `certificate_refused`). */
export type Refusal =
	| 'unreadable'
	| 'no_key'
	| 'pfx_password'
	| 'key_mismatch'
	| 'unsupported_key'
	| 'wrong_name'
	| 'not_yet_valid'
	| 'expired';

export const loadCertificate = () => api<CertificateStatus>('GET', '/api/settings/certificate');
export const uploadPem = (certificate: string, key: string) =>
	api<OwnCertificate>('PUT', '/api/settings/certificate', { certificate, key });
/** `pfx` in base64. */
export const uploadPfx = (pfx: string, password: string) =>
	api<OwnCertificate>('PUT', '/api/settings/certificate', { pfx, password });
export const resetCertificate = () => api('DELETE', '/api/settings/certificate');

/** An own certificate this close to its end gets a warning. */
export const EXPIRY_WARNING_DAYS = 30;

/** Whether `info` runs out within the warning time, or has. */
export const runsOutSoon = (info: CertificateInfo, now = Date.now()) =>
	info.not_after * 1000 - now < EXPIRY_WARNING_DAYS * 24 * 3600 * 1000;
