/**
 * The logins the extension offers, and filling them into a tab (#201,
 * ADR 0017): shared ones as the server lists them, personal ones from the
 * unlocked personal vault, which the extension opens itself.
 */
import { parseTotp, totpCode } from '$lib/vault/totp';
import { readEntries } from '$lib/vault/vault';
import { pageForm, type Form, type Values } from './fill';
import { fillValues, loadLogins, loadPersonal, oneTimeCode } from './server';
import { matches } from './site';
import { vaultKey } from './store';

export interface Login {
	/** Unique across both kinds, for lists. */
	key: string;
	kind: 'shared' | 'personal';
	id: string;
	name: string;
	username: string;
	url: string;
	hasTotp: boolean;
	/** A personal login's secrets, from the unlocked vault; never a shared one's. */
	password?: string;
	totp?: string;
}

/** Whether the user has a personal vault, and whether it is open here. */
export type Personal = 'none' | 'locked' | 'open';

export interface Logins {
	logins: Login[];
	personal: Personal;
	/** A problem code when the shared logins could not be read. */
	error: string | null;
}

export async function allLogins(): Promise<Logins> {
	const [shared, vault] = await Promise.all([loadLogins(), loadPersonal()]);
	const logins: Login[] = shared.ok
		? shared.data.map((login) => ({
				key: `shared:${login.id}`,
				kind: 'shared',
				id: login.id,
				name: login.name,
				username: login.username,
				url: login.url,
				hasTotp: login.has_totp
			}))
		: [];
	let personal: Personal = 'none';
	if (vault.ok && vault.data.unlocks.length > 0) {
		const key = await vaultKey();
		personal = key ? 'open' : 'locked';
		if (key) {
			for (const { id, content } of await readEntries(key, vault.data)) {
				if (!content || content.kind === 'folder' || content.deleted) continue;
				logins.push({
					key: `personal:${id}`,
					kind: 'personal',
					id,
					name: content.title,
					username: content.username,
					url: content.url,
					hasTotp: !!content.totp && parseTotp(content.totp) !== null,
					password: content.password,
					totp: content.totp
				});
			}
		}
	}
	logins.sort((a, b) => a.name.localeCompare(b.name));
	return { logins, personal, error: shared.ok ? null : shared.code };
}

/** The logins that belong to the page at `origin`. */
export const forPage = (logins: Login[], origin: string | null) =>
	origin ? logins.filter((login) => login.url && matches(login.url, origin)) : [];

/** Logins whose name, user name or address contain `query`, in any case. */
export function search(logins: Login[], query: string): Login[] {
	const words = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (words.length === 0) return [];
	return logins.filter((login) => {
		const text = `${login.name} ${login.username} ${login.url}`.toLowerCase();
		return words.every((word) => text.includes(word));
	});
}

/** The origin of the page in a tab; null for anything but http and https. */
export function tabOrigin(tab: chrome.tabs.Tab | undefined): string | null {
	try {
		const url = new URL(tab?.url ?? '');
		return url.protocol === 'https:' || url.protocol === 'http:' ? url.origin : null;
	} catch {
		return null;
	}
}

/** What filling came to: a problem code, or which fields were filled. */
export type Outcome = { ok: true; filled: Form } | { ok: false; code: string };

/**
 * The frame of the tab to fill: the top frame if it holds the form, else
 * the first frame of the same origin that does. Frames of other origins
 * answer nothing (fill.ts), and those of other sites are out of the
 * extension's reach anyway: it may only touch the tab it was asked in.
 */
async function formFrame(
	tabId: number,
	origin: string,
	wanted: (form: Form) => boolean
): Promise<{ frameId: number; form: Form } | null> {
	const results = await chrome.scripting.executeScript({
		target: { tabId, allFrames: true },
		func: pageForm,
		args: [origin, null]
	});
	const found = results
		.filter((r) => r.result && wanted(r.result as Form))
		.sort((a, b) => (a.frameId === 0 ? -1 : b.frameId === 0 ? 1 : 0));
	const first = found[0];
	return first ? { frameId: first.frameId, form: first.result as Form } : null;
}

async function fillFrame(tabId: number, frameId: number, origin: string, values: Values) {
	const [result] = await chrome.scripting.executeScript({
		target: { tabId, frameIds: [frameId] },
		func: pageForm,
		args: [origin, values]
	});
	return (result?.result as Form | null) ?? null;
}

/**
 * Fills `login` into the tab: user name and password, or only the user name
 * on the first page of a two-step sign-in. A shared login's password comes
 * from the server only once a form is there to take it; the server checks
 * the page again and writes the audit entry.
 */
export async function fillLogin(tab: chrome.tabs.Tab, login: Login): Promise<Outcome> {
	const origin = tabOrigin(tab);
	if (tab.id === undefined || !origin || !matches(login.url, origin)) {
		return { ok: false, code: 'wrong_site' };
	}
	let frame;
	try {
		frame = await formFrame(tab.id, origin, (form) => form.user || form.password);
	} catch {
		return { ok: false, code: 'extension_no_access' };
	}
	if (!frame) return { ok: false, code: 'extension_no_form' };
	let values: Values;
	if (!frame.form.password) {
		// The user name is no secret: it stands in the list already.
		values = { username: login.username };
	} else if (login.kind === 'personal') {
		values = { username: login.username, password: login.password ?? '' };
	} else {
		const answer = await fillValues(login.id, origin);
		if (!answer.ok) return { ok: false, code: answer.code };
		values = answer.data;
	}
	const filled = await fillFrame(tab.id, frame.frameId, origin, values);
	return filled ? { ok: true, filled } : { ok: false, code: 'extension_no_form' };
}

/** The current one-time code of a login; for a shared one, audited with its purpose. */
export async function currentCode(
	login: Login,
	purpose: 'fill' | 'copy',
	origin?: string
): Promise<{ ok: true; code: string } | { ok: false; code: string }> {
	if (login.kind === 'personal') {
		const params = login.totp ? parseTotp(login.totp) : null;
		if (!params) return { ok: false, code: 'not_found' };
		return { ok: true, code: await totpCode(params, Math.floor(Date.now() / 1000)) };
	}
	const answer = await oneTimeCode(login.id, purpose, origin);
	return answer.ok ? { ok: true, code: answer.data.code } : { ok: false, code: answer.code };
}

/** Fills the login's one-time code into the tab's code field, or its focused field. */
export async function fillCode(tab: chrome.tabs.Tab, login: Login): Promise<Outcome> {
	const origin = tabOrigin(tab);
	if (tab.id === undefined || !origin || !matches(login.url, origin)) {
		return { ok: false, code: 'wrong_site' };
	}
	let frame;
	try {
		frame = await formFrame(tab.id, origin, (form) => form.code);
	} catch {
		return { ok: false, code: 'extension_no_access' };
	}
	if (!frame) return { ok: false, code: 'extension_no_code_field' };
	const code = await currentCode(login, 'fill', origin);
	if (!code.ok) return code;
	const filled = await fillFrame(tab.id, frame.frameId, origin, { code: code.code });
	return filled ? { ok: true, filled } : { ok: false, code: 'extension_no_code_field' };
}
