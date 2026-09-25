import { expect, test } from '@playwright/test';
import { step, totp } from './totp';

// The setup wizard of a fresh installation (#143). It runs before every other
// test (playwright.config.ts): they rely on alice administering through
// "RH Admins", which the end of this test grants.
//
// The link comes from `remotehub setup-code` (E2E_SETUP_LINK); the CI makes
// one for its fresh database. A database set up before needs none.

const run = Date.now().toString(36);
/** The wizard's page, and not /sign-in/setup. */
const wizard = /^https?:\/\/[^/]+\/setup$/;

test('a fresh installation is set up in the browser', async ({ page }) => {
	const status = (await (await page.request.get('/api/setup')).json()) as { phase: string };
	test.skip(status.phase === 'complete', 'this database is set up already');
	const link = process.env.E2E_SETUP_LINK;
	expect(link, 'E2E_SETUP_LINK: the link `remotehub setup-code` prints').toBeTruthy();

	// Without the code, the wizard only says where to get one.
	await page.goto('/sign-in');
	await expect(page).toHaveURL(wizard);
	await expect(page.getByText('remotehub is waiting for its setup.')).toBeVisible();

	const setupLink = new URL(link!);
	await page.goto(setupLink.pathname + setupLink.hash);
	// The code leaves the address bar at once.
	await expect(page).toHaveURL(wizard);
	await page.getByLabel('Name').fill('Sam Setup');
	await page.getByLabel('E-mail').fill(`setup-${run}@remotehub.test`);
	await page.getByRole('button', { name: 'Create administrator' }).click();

	// The invitation's code goes by itself; the password and the app follow.
	await page.getByLabel('New password').fill(`Setup-Passw0rd-${run}`);
	await page.getByRole('button', { name: 'Continue' }).click();
	const secret = (await page.getByTestId('totp-secret').innerText()).trim();
	await page.getByLabel('Code from the app').fill(totp(secret, step()));
	await page.getByRole('button', { name: 'Set up' }).click();

	// Signed in, the administrator is back in the wizard.
	await expect(page).toHaveURL(wizard);
	await expect(page.getByRole('heading', { name: 'Break-glass account' })).toBeVisible();
	await page.getByRole('button', { name: 'Create break-glass account' }).click();
	await expect(page.getByTestId('break-glass-username')).toHaveText('emergency');
	await expect(page.getByTestId('break-glass-password')).toHaveText(/^\S{24}$/);
	await expect(page.getByRole('img', { name: 'QR code for the authenticator app' })).toBeVisible();
	const onward = page.getByRole('button', { name: 'Continue' });
	await expect(onward).toBeDisabled();
	await page.getByLabel('Printed and stored').check();
	await onward.click();

	await expect(page.getByRole('heading', { name: 'Recovery key' })).toBeVisible();
	await page.getByLabel('Passphrase of the key file').fill('setup key file passphrase');
	await page.getByLabel('Passphrase again').fill('setup key file passphrase');
	const downloading = page.waitForEvent('download');
	await page.getByRole('button', { name: 'Create a recovery key' }).click();
	await downloading;
	await expect(page.getByTestId('organisation-private-key')).toHaveText(/[A-Z2-7]{4}-/);
	await page.getByRole('button', { name: 'I have kept it safe' }).click();

	await expect(page.getByRole('heading', { name: 'Ready' })).toBeVisible();
	await page.getByRole('button', { name: 'Finish' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
	// Nothing was left out, so nothing reminds of it.
	await expect(page.getByText('There is no break-glass account.')).toHaveCount(0);

	// The directory's administrators: the group RH Admins.
	const origin = { origin: new URL(page.url()).origin };
	const found = (await (
		await page.request.get('/api/directory/principals?q=RH%20Admins')
	).json()) as { kind: string; sid: string; name: string }[];
	const admins = found.find((principal) => principal.name === 'RH Admins');
	expect(admins, 'the lab has the group RH Admins').toBeTruthy();
	const granted = await page.request.put(`/api/roles/administrator/members/${admins!.sid}`, {
		headers: origin,
		data: { principal_kind: 'group', principal_name: 'RH Admins' }
	});
	expect(granted.status()).toBe(204);

	// Setup is over for good: the link leads nowhere now.
	await page.goto(setupLink.pathname + setupLink.hash);
	await expect(page).toHaveURL(/^https?:\/\/[^/]+\/$/);
});
