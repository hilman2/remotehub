import { expect, test, type Page } from '@playwright/test';
import { join } from 'node:path';

// Connections end to end: an AD user of the test lab signs in, sets up
// folders, credentials and devices through the UI, and works on them in the
// browser — SSH in a terminal, RDP as a picture — against the test lab.

// The development database keeps data between runs, so names are unique.
const run = Date.now().toString(36);
const sshHost = process.env.E2E_SSH_HOST ?? 'ssh-target';
const desktopHost = process.env.E2E_DESKTOP_HOST ?? 'desktop-target';
/** Keys of the lab's SSH target; the tests run in web/. */
const labKeys = join(process.cwd(), '..', 'deploy', 'testlab', 'ssh');

async function signIn(page: Page, user = 'alice', password = 'Alice-Passw0rd!') {
	await page.goto('/');
	await expect(page).toHaveURL(/\/sign-in$/);
	await page.getByLabel('User name').fill(user);
	await page.getByLabel('Password').fill(password);
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

/** A device in the folder that signs in with the credential (SSH by default). */
async function newDevice(
	page: Page,
	folder: string,
	name: string,
	credential: string,
	protocol?: { label: string; host: string }
) {
	const dialog = page.getByRole('dialog');
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	if (protocol) await dialog.getByLabel('Protocol').selectOption({ label: protocol.label });
	await dialog.getByLabel('Host name or IP address').fill(protocol?.host ?? sshHost);
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

test('an SSH device signs in with a certificate from remotehub', async ({ page, context }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E ca ${run}`;
	await newFolder(page, folder);

	// No credential at all: remotehub signs a key for alice at every connection.
	const name = `lab ssh ca ${run}`;
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog
		.getByLabel('Sign in with')
		.selectOption({ label: 'A certificate from remotehub, as myself' });
	await expect(dialog.getByText(/\/api\/ssh-ca\.pub/)).toBeVisible();
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();

	const [terminal] = await Promise.all([
		context.waitForEvent('page'),
		page.getByRole('link', { name: 'Connect' }).click()
	]);
	await terminal.locator('.xterm').click();
	await terminal.keyboard.type('echo "ca says $(whoami)"');
	await terminal.keyboard.press('Enter');
	await expect(terminal.locator('.xterm-rows')).toContainText('ca says alice');
	await terminal.close();
});

test('access asked for just in time is approved by someone else', async ({ page, browser }) => {
	// alice sets up a device that bob may only see.
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E jit ${run}`;
	await newFolder(page, folder);
	const name = `lab ssh jit ${run}`;
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();
	await page.getByRole('button', { name: 'Permissions' }).click();
	await dialog.getByLabel('Search users and groups').fill('Bob');
	await dialog.getByRole('button', { name: /Bob Helpdesk/ }).click();
	await dialog.locator('select').selectOption({ label: 'See' });
	await dialog.getByRole('button', { name: 'Grant' }).click();
	await expect(dialog.getByText('Bob Helpdesk')).toBeVisible();
	await page.keyboard.press('Escape');

	// bob asks for connect with a reason.
	const bobs = await browser.newContext();
	const bob = await bobs.newPage();
	await signIn(bob, 'bob', 'Bob-Passw0rd!');
	// A device's entry in the tree shows its host, too.
	const entry = bob.getByRole('tree').getByRole('button', { name: `${name} ${sshHost}` });
	await bob.getByRole('searchbox').fill(name);
	await entry.click();
	await expect(bob.getByRole('link', { name: 'Connect' })).toHaveCount(0);
	await bob.getByRole('button', { name: 'Request access' }).click();
	await bob.getByRole('dialog').getByLabel('Reason').fill('Rotate the logs');
	await bob.getByRole('dialog').getByRole('button', { name: 'Send request' }).click();
	await expect(bob.getByText('Request sent.')).toBeVisible();

	// alice approves it on the requests page.
	await page.getByRole('link', { name: 'Access requests' }).click();
	const item = page.getByRole('listitem').filter({ hasText: 'Rotate the logs' });
	await item.getByRole('button', { name: 'Approve' }).click();
	await expect(item).toHaveCount(0);

	// bob may connect now, and sees until when.
	await bob.reload();
	await bob.getByRole('searchbox').fill(name);
	await entry.click();
	await expect(bob.getByRole('link', { name: 'Connect' })).toBeVisible();
	await bob.getByRole('link', { name: 'Access requests' }).click();
	await expect(bob.getByText(/Approved · until .* · by alice/)).toBeVisible();
	await bobs.close();
});

test('an RDP desktop opens with the password LAPS keeps in the directory', async ({
	page,
	context
}) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E laps ${run}`;
	await newFolder(page, folder);
	// No credential: the lab's directory holds a LAPS password for desktop-target.
	const name = `lab rdp laps ${run}`;
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Protocol').selectOption({ label: 'Remote Desktop (RDP)' });
	await dialog.getByLabel('Host name or IP address').fill(desktopHost);
	await dialog
		.getByLabel('Sign in with')
		.selectOption({ label: 'The local administrator from LAPS' });
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();

	const [desktop] = await Promise.all([
		context.waitForEvent('page'),
		page.getByRole('link', { name: 'Connect' }).click()
	]);
	await expect(desktop.getByRole('application')).toBeVisible();
	// The lab desktop's background (#1e5b8c): the sign-in worked.
	await expect
		.poll(
			() =>
				desktop.evaluate(() => {
					const canvas = document.querySelector<HTMLCanvasElement>('[role=application] canvas');
					const pixel = canvas?.getContext('2d')?.getImageData(4, 4, 1, 1).data;
					return pixel ? [pixel[0], pixel[1], pixel[2]] : null;
				}),
			{ timeout: 15_000 }
		)
		.toEqual([30, 91, 140]);
	await desktop.close();
});

test('an RDP desktop opens in the browser', async ({ page, context }) => {
	// Chromium asks before a page reads the clipboard; the test says yes.
	await context.grantPermissions(['clipboard-read', 'clipboard-write']);
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E desktops ${run}`;
	await newFolder(page, folder);

	const credential = `desktop tester ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Password', { exact: true }).fill('Tester-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();

	await newDevice(page, folder, `lab rdp ${run}`, credential, {
		label: 'Remote Desktop (RDP)',
		host: desktopHost
	});
	const [desktop] = await Promise.all([
		context.waitForEvent('page'),
		page.getByRole('link', { name: 'Connect' }).click()
	]);
	// The first connection pins the device's certificate.
	await expect(desktop.getByText(/the certificate .* is now pinned/)).toBeVisible();
	await expect(desktop.getByRole('application')).toBeVisible();

	// The lab desktop's background (#1e5b8c) in the corner of the picture.
	await expect
		.poll(
			() =>
				desktop.evaluate(() => {
					const canvas = document.querySelector<HTMLCanvasElement>('[role=application] canvas');
					const pixel = canvas?.getContext('2d')?.getImageData(4, 4, 1, 1).data;
					return pixel ? [pixel[0], pixel[1], pixel[2]] : null;
				}),
			{ timeout: 15_000 }
		)
		.toEqual([30, 91, 140]);

	// The clipboard, both ways: the lab desktop answers every text that
	// reaches its clipboard with `echo:<text>`, which comes back to the
	// browser's clipboard. First what was copied before the view had focus…
	const clipboard = () => desktop.evaluate(() => navigator.clipboard.readText());
	const copy = (text: string) =>
		desktop.evaluate((text) => navigator.clipboard.writeText(text), text);
	await copy('copied in the browser');
	await desktop.getByRole('application').click();
	await expect.poll(clipboard).toBe('echo:copied in the browser');
	// …then what was copied while it had focus, with the paste key.
	await copy('pasted with Ctrl+V');
	await desktop.keyboard.press('Control+V');
	await expect.poll(clipboard).toBe('echo:pasted with Ctrl+V');
	await desktop.close();
});
