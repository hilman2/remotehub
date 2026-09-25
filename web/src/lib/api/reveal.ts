/** Showing and copying stored credentials (crates/server/src/api/reveal.rs, #97). */
import { api } from './client';

/** What a credential or a device's own credentials hold. */
export interface Revealed {
	username: string;
	domain: string;
	password?: string;
	private_key?: string;
	passphrase?: string;
	certificate?: string;
	/** A credential's protected custom fields (#98). */
	fields?: { name: string; value: string }[];
}

/** Why it is revealed; the audit log keeps it. */
export type RevealPurpose = 'show' | 'copy';

export const reveal = (owner: 'credentials' | 'devices', id: string, purpose: RevealPurpose) =>
	api<Revealed>('POST', `/api/${owner}/${id}/reveal`, { purpose });
