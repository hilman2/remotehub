/**
 * Ory Kratos' self-service flows (#103), reached through remotehub under
 * /api/auth (crates/server/src/api/accounts.rs). Kratos describes each form
 * as nodes; remotehub renders them itself and submits them as JSON.
 */
import { m } from '$lib/paraglide/messages';

/** A text of Kratos: its `id` says what it means, `text` is English. */
export interface UiText {
	id: number;
	text: string;
	type: 'info' | 'error' | 'success';
	context?: Record<string, unknown>;
}

export interface UiNode {
	type: 'input' | 'text' | 'img' | 'a' | 'script' | 'div';
	group: string;
	attributes: {
		name?: string;
		type?: string;
		value?: unknown;
		id?: string;
		src?: string;
		text?: UiText;
	};
	messages: UiText[];
}

export interface Flow {
	id: string;
	/** Settings flows: `success` once a change is saved. */
	state?: string;
	ui: { action: string; nodes: UiNode[]; messages?: UiText[] };
}

export type FlowKind = 'login' | 'settings' | 'recovery';

/** How a flow request ended. */
export type FlowResult =
	/** A form to show, with its messages (also after a failed submit). */
	| { kind: 'flow'; flow: Flow }
	/** A sign-in is done. */
	| { kind: 'done' }
	/** Kratos sends the browser elsewhere: the next flow, or a second factor. */
	| { kind: 'redirect'; to: URL }
	/** The flow ran out; start a new one. */
	| { kind: 'expired' }
	| { kind: 'failed' };

const BASE = '/api/auth';

async function call(path: string, init: RequestInit = {}): Promise<FlowResult> {
	let response: Response;
	try {
		response = await fetch(path, {
			...init,
			credentials: 'same-origin',
			headers: {
				accept: 'application/json',
				...(init.body ? { 'content-type': 'application/json' } : {})
			}
		});
	} catch {
		return { kind: 'failed' };
	}
	const body = (await response.json().catch(() => null)) as Record<string, unknown> | null;
	const redirect = body?.redirect_browser_to;
	if (typeof redirect === 'string')
		return { kind: 'redirect', to: new URL(redirect, location.href) };
	if (response.status === 410) return { kind: 'expired' };
	if (body && 'ui' in body) return { kind: 'flow', flow: body as unknown as Flow };
	if (response.ok) return { kind: 'done' };
	return { kind: 'failed' };
}

/** Starts a flow; `query` e.g. `?aal=aal2` for the second factor. */
export const startFlow = (kind: FlowKind, query = '') =>
	call(`${BASE}/self-service/${kind}/browser${query}`);

export const loadFlow = (kind: FlowKind, id: string) =>
	call(`${BASE}/self-service/${kind}/flows?id=${encodeURIComponent(id)}`);

/**
 * Submits `values` to the flow, with its CSRF token. Kratos names the
 * absolute URL of remotehub's proxy; only its path and query are used, so
 * the request stays on this origin.
 */
export function submitFlow(flow: Flow, values: Record<string, unknown>): Promise<FlowResult> {
	const action = new URL(flow.ui.action, location.href);
	return call(action.pathname + action.search, {
		method: 'POST',
		body: JSON.stringify({ ...values, csrf_token: value(flow, 'csrf_token') })
	});
}

/**
 * Ends the Kratos session in this browser, if there is one. A sign-in with
 * typed credentials starts from here: otherwise a session left behind would
 * sign in whoever types anything.
 */
export async function endSession(): Promise<void> {
	const headers = { accept: 'application/json' };
	try {
		const response = await fetch(`${BASE}/self-service/logout/browser`, {
			headers,
			credentials: 'same-origin'
		});
		if (!response.ok) return;
		const { logout_url } = (await response.json()) as { logout_url: string };
		const url = new URL(logout_url, location.href);
		await fetch(url.pathname + url.search, { headers, credentials: 'same-origin' });
	} catch {
		// Kratos away: the sign-in that follows fails on its own.
	}
}

/** The value of the input node `name`, if the flow has it. */
export function value(flow: Flow, name: string): unknown {
	return flow.ui.nodes.find((node) => node.attributes.name === name)?.attributes.value;
}

/** Whether the flow offers the input node `name`. */
export const offers = (flow: Flow, name: string) =>
	flow.ui.nodes.some((node) => node.attributes.name === name);

/** A node by its `id` attribute (texts and images). */
export const node = (flow: Flow, id: string) => flow.ui.nodes.find((n) => n.attributes.id === id);

/** Every message of the flow: its own and its nodes'. */
export function messages(flow: Flow): UiText[] {
	return [...(flow.ui.messages ?? []), ...flow.ui.nodes.flatMap((n) => n.messages)];
}

/** The flow ID a redirect names, e.g. `/sign-in/setup?flow=…`. */
export const flowId = (to: URL) => to.searchParams.get('flow');

/**
 * Kratos' text in the UI's language where remotehub knows it
 * (https://www.ory.com/docs/kratos/concepts/ui-messages); Kratos' English
 * text otherwise.
 */
export function kratosText(text: UiText): string {
	switch (text.id) {
		case 4000006:
			return m.error_invalid_credentials();
		case 4000008:
			return m.kratos_code_invalid();
		case 4060006:
			return m.kratos_recovery_code_invalid();
		case 1050001:
			return m.kratos_saved();
		default:
			return text.text;
	}
}
