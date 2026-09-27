/** Showing and copying stored credentials (crates/server/src/api/reveal.rs, #97). */
import { NETWORK_ERROR, api, type ApiResult } from './client';
import { readProblem } from './errors';

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
	/** A credential's one-time password, for an export only (#193). */
	totp?: string;
}

/** A one-time code the server computed (#193). */
export interface Code {
	code: string;
	/** Seconds it still holds. */
	remaining: number;
	period: number;
}

/** The current one-time code of a shared credential; audited like a reveal. */
export const credentialCode = (id: string, purpose: 'show' | 'copy') =>
	api<Code>('POST', `/api/credentials/${id}/code`, { purpose });

/** Why it is revealed; the audit log keeps it. */
export type RevealPurpose = 'show' | 'copy' | 'export';

export const reveal = (
	owner: 'credentials' | 'devices' | 'profiles',
	id: string,
	purpose: RevealPurpose,
	version?: number
) => api<Revealed>('POST', `/api/${owner}/${id}/reveal`, { purpose, version });

/** A version of a credential's secrets (#100). */
export interface Version {
	version: number;
	/** RFC 3339, UTC. */
	created_at: string;
}

export const loadVersions = (id: string) =>
	api<Version[]>('GET', `/api/credentials/${id}/versions`);

/** Where a credential's file is downloaded; the server records it. */
export const attachmentUrl = (credential: string, attachment: string) =>
	`/api/credentials/${credential}/attachments/${attachment}`;

/** A credential's file for the vault's viewer (#200), recorded as viewed; null if refused. */
export async function viewAttachment(credential: string, attachment: string) {
	try {
		const response = await fetch(`${attachmentUrl(credential, attachment)}?purpose=view`, {
			credentials: 'same-origin'
		});
		return response.ok ? await response.blob() : null;
	} catch {
		return null;
	}
}

/** Sends a file as it is; one of the same name is replaced. */
export async function uploadAttachment(
	credential: string,
	file: File
): Promise<ApiResult<unknown>> {
	let response: Response;
	try {
		response = await fetch(
			`/api/credentials/${credential}/attachments?name=${encodeURIComponent(file.name)}`,
			{
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': file.type || 'application/octet-stream' },
				body: file
			}
		);
	} catch {
		return { ok: false, status: 0, code: NETWORK_ERROR, params: {} };
	}
	if (response.ok) return { ok: true, data: undefined };
	const problem = await readProblem(response);
	return {
		ok: false,
		status: response.status,
		code: problem?.code ?? 'internal',
		params: problem?.params ?? {}
	};
}

export const deleteAttachment = (credential: string, attachment: string) =>
	api('DELETE', attachmentUrl(credential, attachment));
