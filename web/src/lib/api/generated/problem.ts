// Generated from crates/server/src/api/problem.rs — do not edit.
// Regenerate: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated

export const ERROR_CODES = [
	'not_found',
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
