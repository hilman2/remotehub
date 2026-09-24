import { expect, test } from '@playwright/test';

// The first connection end to end (M1): an AD user of the test lab signs
// in, sets up a folder, a credential and an SSH device through the UI, works
// in the device's terminal and finds the connection in the audit log.

// The development database keeps data between runs, so names are unique.
const run = Date.now().toString(36);
const sshHost = process.env.E2E_SSH_HOST ?? 'ssh-target';

test('an AD user adds an SSH device and works in its terminal', async ({ page, context }) => {
	await page.goto('/');
	await expect(page).toHaveURL(/\/sign-in$/);
	await page.getByLabel('User name').fill('alice');
	await page.getByLabel('Password').fill('Alice-Passw0rd!');
	await page.getByRole('button', { name: 'Sign in' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();

	const dialog = page.getByRole('dialog');
	const tree = page.getByRole('tree');

	// A folder at the top.
	const folder = `E2E ${run}`;
	await page.getByRole('button', { name: 'New folder' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(folder);
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: folder })).toBeVisible();

	// A credential in it; the password is never shown again.
	const credential = `tester ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Password', { exact: true }).fill('Tester-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();
	await expect(page.locator('body')).not.toContainText('Tester-Passw0rd!');

	// An SSH device that signs in with it.
	await tree.getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(`lab ssh ${run}`);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog.getByLabel('Sign in with').selectOption({ label: 'A stored credential' });
	await dialog
		.getByLabel('Credential', { exact: true })
		.selectOption({ label: `${credential} · tester` });
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: `lab ssh ${run}` })).toBeVisible();

	// Its terminal opens in a new tab; the first connection pins the host key.
	const [terminal] = await Promise.all([
		context.waitForEvent('page'),
		page.getByRole('link', { name: 'Connect' }).click()
	]);
	await expect(terminal.getByText(/is now pinned/)).toBeVisible();
	await terminal.locator('.xterm').click();
	await terminal.keyboard.type('echo "e2e says $(whoami)"');
	await terminal.keyboard.press('Enter');
	await expect(terminal.locator('.xterm-rows')).toContainText('e2e says tester');
	await terminal.close();

	// The connection is in the audit log.
	await page.getByRole('link', { name: 'Audit log' }).click();
	await expect(page.getByText('Opened a connection').first()).toBeVisible();
});
