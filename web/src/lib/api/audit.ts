/** The audit log (crates/server/src/audit.rs). */
import type { Locale } from '$lib/i18n';
import { m } from '$lib/paraglide/messages';
import { api } from './client';
import { AUDIT_ACTIONS, type AuditAction } from './generated/audit';

export { AUDIT_ACTIONS, type AuditAction };

export interface AuditRecord {
	seq: number;
	/** RFC 3339, UTC */
	at: string;
	actor_name: string;
	action: string;
	object_type: string | null;
	object_id: string | null;
	details: Record<string, unknown>;
	address: string | null;
}

export interface Verification {
	entries: number;
	first_broken: number | null;
}

/** `session.sign_in` → `session_sign_in`, at the type level too. */
type Underscored<A extends string> = A extends `${infer Head}.${infer Tail}`
	? `${Head}_${Underscored<Tail>}`
	: A;

function messageKey<A extends AuditAction>(action: A): `audit_${Underscored<A>}` {
	return `audit_${action.replaceAll('.', '_')}` as `audit_${Underscored<A>}`;
}

export function isAuditAction(action: string): action is AuditAction {
	return (AUDIT_ACTIONS as readonly string[]).includes(action);
}

/** The label of an action; unknown actions (newer server) show their name. */
export function auditActionLabel(action: string, locale?: Locale): string {
	if (!isAuditAction(action)) return action;
	return m[messageKey(action)]({}, locale ? { locale } : undefined);
}

export const loadAudit = (before?: number) =>
	api<AuditRecord[]>('GET', `/api/audit?limit=100${before ? `&before=${before}` : ''}`);

export const verifyAudit = () => api<Verification>('POST', '/api/audit/verify');
