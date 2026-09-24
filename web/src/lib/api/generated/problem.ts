// Generated from crates/server/src/api/problem.rs — do not edit.
// Regenerate: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated

export const ERROR_CODES = [
	'not_found',
	'invalid_request',
	'unauthenticated',
	'forbidden',
	'invalid_credentials',
	'account_disabled',
	'account_locked',
	'account_expired',
	'password_change_required',
	'too_many_attempts',
	'forbidden_origin',
	'name_taken',
	'folder_not_empty',
	'host_key_changed',
	'certificate_changed',
	'tls_required',
	'target_unreachable',
	'target_auth_failed',
	'connection_failed',
	'own_account_unavailable',
	'ssh_ca_unavailable',
	'directory_unavailable',
	'database_unavailable',
	'internal'
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];

/** RFC 9457 problem response of the API. */
export interface Problem {
	type: string;
	title: string;
	status: number;
	code: ErrorCode;
	params?: Record<string, unknown>;
}
