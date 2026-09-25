/** The mail server (crates/server/src/api/mail.rs, #145). */
import { api } from './client';

export type Security = 'tls' | 'starttls' | 'none';

/** Everything about the server but its password. */
export interface MailServer {
	host: string;
	port: number;
	security: Security;
	username: string | null;
	from_address: string;
	from_name: string;
	ca_pem: string | null;
}

/** What the form sends: no password keeps the stored one. */
export interface MailInput extends MailServer {
	password: string | null;
}

export type SendStep =
	'not_configured' | 'address' | 'connect' | 'tls' | 'sign_in' | 'rejected' | 'other';

export interface SendFailure {
	step: SendStep;
	/** The server's answer or the library's words. */
	detail: string;
}

export const loadMailServer = () => api<{ server: MailServer | null }>('GET', '/api/settings/mail');
export const saveMailServer = (input: MailInput) => api('PUT', '/api/settings/mail', input);
export const removeMailServer = () => api('DELETE', '/api/settings/mail');
export const sendTestMail = (input: MailInput, to: string, language: string) =>
	api<{ failure?: SendFailure }>('POST', '/api/settings/mail/test', { ...input, to, language });

/** The port each kind of security usually has. */
export const DEFAULT_PORTS: Record<Security, number> = { tls: 465, starttls: 587, none: 25 };
