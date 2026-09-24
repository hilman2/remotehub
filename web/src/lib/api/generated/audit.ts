// Generated from crates/server/src/audit.rs — do not edit.
// Regenerate: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated

export const AUDIT_ACTIONS = [
	'session.sign_in',
	'session.sign_in_failed',
	'session.sign_out',
	'audit.verified'
] as const;

export type AuditAction = (typeof AUDIT_ACTIONS)[number];
