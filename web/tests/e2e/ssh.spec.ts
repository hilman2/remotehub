import { expect, test, type Page } from '@playwright/test';
import { join } from 'node:path';

// The first connection end to end (M1): an AD user of the test lab signs
// in, sets up a folder, a credential and an SSH device through the UI, works
// in the device's terminal and finds the connection in the audit log.

// The development database keeps data between runs, so names are unique.
const run = Date.now().toString(36);
const sshHost = process.env.E2E_SSH_HOST ?? 'ssh-target';
/** Keys of the lab's SSH target; the tests run in web/. */
const labKeys = join(process.cwd(), '..', 'deploy', 'testlab', 'ssh');

async function signIn(page: Page) {
	await page.goto('/');
	await expect(page).toHaveURL(/\/sign-in$/);
	await page.getByLabel('User name').fill('alice');
	await page.getByLabel('Password').fill('Alice-Passw0rd!');
	await page.getByRole('button', { name: 'Sign in' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
}

async function newFolder(page: Page, name: string) {
	const dialog = page.getByRole('dialog');
	await page.getByRole('button', { name: 'New folder' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();
}

/** An SSH device in the folder that signs in with the credential. */
async function newDevice(page: Page, folder: string, name: string, credential: string) {
	const dialog = page.getByRole('dialog');
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog.getByLabel('Sign in with').selectOption({ label: 'A stored credential' });
	await dialog
		.getByLabel('Credential', { exact: true })
		.selectOption({ label: `${credential} · tester` });
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();
}

test('an AD user adds an SSH device and works in its terminal', async ({ page, context }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');

	// A folder at the top.
	const folder = `E2E ${run}`;
	await newFolder(page, folder);

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
	await newDevice(page, folder, `lab ssh ${run}`, credential);

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

test('an SSH key protected by a passphrase signs in', async ({ page, context }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E keys ${run}`;
	await newFolder(page, folder);

	// The key comes from a file. Without its passphrase it is refused, and the
	// form keeps the key for the second try.
	const credential = `tester key ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('Type').selectOption({ label: 'SSH key' });
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog
		.getByLabel('Load key from file')
		.setInputFiles(join(labKeys, 'tester_ed25519_passphrase'));
	await expect(dialog.getByLabel('Private key')).toHaveValue(/BEGIN OPENSSH PRIVATE KEY/);
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(dialog.getByRole('alert')).toHaveText("The key's passphrase is missing or wrong.");
	await dialog.getByLabel('Passphrase').fill('Key-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();

	// Shown by its fingerprint, never by the key.
	await expect(page.getByText(/^SHA256:/)).toBeVisible();
	await expect(page.locator('body')).not.toContainText('PRIVATE KEY');

	await newDevice(page, folder, `lab ssh key ${run}`, credential);
	const [terminal] = await Promise.all([
		context.waitForEvent('page'),
		page.getByRole('link', { name: 'Connect' }).click()
	]);
	await terminal.locator('.xterm').click();
	await terminal.keyboard.type('echo "key says $(whoami)"');
	await terminal.keyboard.press('Enter');
	await expect(terminal.locator('.xterm-rows')).toContainText('key says tester');
	await terminal.close();
});
