import { expect, test } from '@playwright/test';
import { inbox } from './mailpit';
import { step, totp } from './totp';

// The setup wizard of a fresh installation (#143). It runs before every other
// test (playwright.config.ts): they rely on the lab's directory (#144), on
// alice administering through "RH Admins", and on the lab's Mailpit as mail
// server (#145), which the wizard sets up here.
//
// The link comes from `remotehub setup-code` (E2E_SETUP_LINK); the CI makes
// one for its fresh database. A database set up before needs none.

const run = Date.now().toString(36);
const administrator = `setup-${run}@remotehub.test`;
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
	await page.getByLabel('E-mail').fill(administrator);
	await page.getByRole('button', { name: 'Create administrator' }).click();

	// The invitation's code goes by itself; the password and the app follow.
	await page.getByLabel('New password').fill(`Setup-Passw0rd-${run}`);
	await page.getByRole('button', { name: 'Continue' }).click();
	const secret = (await page.getByTestId('totp-secret').innerText()).trim();
	await page.getByLabel('Code from the app').fill(totp(secret, step()));
	await page.getByRole('button', { name: 'Set up' }).click();

	// Signed in, the administrator is back in the wizard: first the lab's
	// directory. Its domain controller presents only its own certificate,
	// which the system does not know; trusted as it is, the check passes.
	await expect(page).toHaveURL(wizard);
	await expect(page.getByRole('heading', { name: 'Directory' })).toBeVisible();
	await page.getByLabel('Server address').fill('ldaps://dc.remotehub.test');
	await page.getByLabel('Service account', { exact: true }).fill('svc-remotehub@remotehub.test');
	await page.getByLabel('Password of the service account').fill('Svc-Passw0rd!');
	await page.getByLabel('Base DN').fill('DC=remotehub,DC=test');
	await page.getByRole('button', { name: 'Test connection' }).click();
	await expect(page.getByTestId('directory-failure')).toContainText(
		'The certificate comes from an unknown CA.'
	);
	await expect(page.getByTestId('directory-fingerprint')).toHaveText(
		/^([0-9A-F]{2}:){31}[0-9A-F]{2}$/
	);
	await page.getByRole('button', { name: 'Trust this certificate' }).click();
	await expect(page.getByTestId('directory-found')).toContainText('Connected:');
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByTestId('directory-found')).toContainText('Saved');

	// The directory's administrators: the group RH Admins.
	await page.getByLabel('Search users and groups').fill('RH Admins');
	await page.getByRole('button', { name: /RH Admins/ }).click();
	await page.getByRole('button', { name: 'Add', exact: true }).click();
	await expect(page.getByRole('list', { name: 'Administrators' })).toContainText('RH Admins');
	await page.getByRole('button', { name: 'Continue' }).click();

	// Then the lab's Mailpit as mail server (#145), with a test mail to the
	// administrator first.
	await expect(page.getByRole('heading', { name: 'Mail', exact: true })).toBeVisible();
	await page.getByLabel('Mail server').fill('mail');
	await page.getByLabel('None').check();
	await page.getByLabel('Port', { exact: true }).fill('1025');
	await page.getByLabel('Sender address').fill('remotehub@remotehub.test');
	await expect(page.getByLabel('Test mail to')).toHaveValue(administrator);
	await page.getByRole('button', { name: 'Send test mail' }).click();
	await expect(page.getByTestId('mail-done')).toContainText('The test mail went to');
	expect((await inbox(administrator)).map((mail) => mail.subject)).toEqual(['remotehub test mail']);
	await page.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByTestId('mail-done')).toContainText('Saved.');
	await page.getByRole('button', { name: 'Continue' }).click();

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

	// Setup is over for good: the link leads nowhere now.
	await page.goto(setupLink.pathname + setupLink.hash);
	await expect(page).toHaveURL(/^https?:\/\/[^/]+\/$/);
});
