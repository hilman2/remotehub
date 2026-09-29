import { expect, type Page } from '@playwright/test';
import { step, totp } from './totp';

// Local accounts through Ory Kratos (#103): invited through Kratos' admin
// API, as `remotehub account invite` does, then set up in the browser.

/** Kratos' admin API; the tests reach it on the compose network. */
export const kratosAdmin = process.env.E2E_KRATOS_ADMIN_URL ?? 'http://kratos:4434';
export const run = Date.now().toString(36);

/** What `remotehub account invite` does: an identity and its one-time code. */
export async function invite(email: string, name: string) {
	const created = await fetch(`${kratosAdmin}/admin/identities`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ schema_id: 'user', traits: { email, name } })
	});
	expect(created.status).toBe(201);
	const { id } = (await created.json()) as { id: string };
	const code = await fetch(`${kratosAdmin}/admin/recovery/code`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ identity_id: id, expires_in: '1h' })
	});
	expect(code.status).toBe(201);
	const invitation = (await code.json()) as { recovery_link: string; recovery_code: string };
	return { id, ...invitation };
}

/** An address no other test, and no repetition, uses. */
export const address = (who: string) =>
	`${who}-${run}-${crypto.randomUUID().slice(0, 8)}@remotehub.test`;

/**
 * Invites an account and walks through the invitation: code, password,
 * authenticator app. Returns the app's key; the account is signed in.
 */
export async function onboard(page: Page, email: string, password: string, name = '') {
	const invitation = await invite(email, name);
	return accept(page, invitation.recovery_link, invitation.recovery_code, password);
}

/** Walks through an invitation's link and code; see `onboard`. */
export async function accept(page: Page, recoveryLink: string, code: string, password: string) {
	const link = new URL(recoveryLink);
	await page.goto(link.pathname + link.search);
	await page.getByLabel('Code', { exact: true }).fill(code);
	await page.getByRole('button', { name: 'Continue' }).click();
	await page.getByLabel('New password').fill(password);
	await page.getByRole('button', { name: 'Continue' }).click();
	// remotehub requires an authenticator app before the first session.
	const secret = (await page.getByTestId('totp-secret').innerText()).trim();
	await page.getByLabel('Code from the app').fill(totp(secret, step()));
	await page.getByRole('button', { name: 'Set up' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
	return secret;
}
