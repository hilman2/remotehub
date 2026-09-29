import { expect, type Browser, type Page } from '@playwright/test';
import { accept, address, invite, run } from './local';

// Showing a secret takes a confirmation with the second factor (#242). The
// lab's directory users have none, and one for alice would stop the other
// tests from signing in as her, in parallel. Tests that show secrets act
// as a local account of their own instead: an administrator with a passkey.

/**
 * Signs `page` in as a new local account with the administrator role and a
 * passkey in a virtual authenticator that confirms the person, as Windows
 * Hello does. Returns its name.
 */
export async function signInConfirming(page: Page, browser: Browser, name = 'Cora Confirm') {
	const email = address('confirm');
	const invitation = await invite(email, name);
	await accept(page, invitation.recovery_link, invitation.recovery_code, `Confirm-Passw0rd-${run}`);

	// alice gives it the administrator role.
	const admins = await browser.newContext();
	const admin = await admins.newPage();
	await admin.goto('/sign-in');
	await admin.getByLabel('User name or e-mail').fill('alice');
	await admin.getByLabel('Password').fill('Alice-Passw0rd!');
	await admin.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(admin.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
	const status = await admin.evaluate(
		async ({ id, name }) =>
			(
				await fetch(`/api/roles/administrator/members/local:${id}`, {
					method: 'PUT',
					headers: { 'content-type': 'application/json' },
					body: JSON.stringify({ principal_kind: 'user', principal_name: name })
				})
			).status,
		{ id: invitation.id, name }
	);
	expect(status).toBe(204);
	await admins.close();

	const cdp = await page.context().newCDPSession(page);
	await cdp.send('WebAuthn.enable');
	await cdp.send('WebAuthn.addVirtualAuthenticator', {
		options: {
			protocol: 'ctap2',
			transport: 'internal',
			hasResidentKey: true,
			hasUserVerification: true,
			isUserVerified: true,
			automaticPresenceSimulation: true
		}
	});
	await page.goto('/account');
	const keys = page.getByRole('region', { name: 'Passkeys and security keys for signing in' });
	await keys.getByLabel('Name of the key').fill('Windows Hello');
	await keys.getByRole('button', { name: 'Add a security key' }).click();
	await expect(keys.getByRole('button', { name: 'Remove Windows Hello' })).toBeVisible();
	await page.goto('/');
	await expect(page.getByRole('link', { name: 'Settings', exact: true })).toBeVisible();
	return name;
}

/** Answers the dialog that asks for the second factor, with the passkey. */
export async function confirmIt(page: Page) {
	const dialog = page.getByRole('dialog').filter({ hasText: 'Confirm that it is you' });
	await dialog.getByRole('button', { name: 'Confirm with passkey' }).click();
	await expect(dialog).toBeHidden();
}
