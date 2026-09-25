import { expect, test, type Page } from '@playwright/test';
import { join } from 'node:path';

// Connections end to end: an AD user of the test lab signs in, sets up
// folders, credentials and devices through the UI, and works on them in the
// browser — SSH in a terminal, RDP and web interfaces as a picture — against
// the test lab.

// The development database keeps data between runs, so names are unique.
const run = Date.now().toString(36);
const sshHost = process.env.E2E_SSH_HOST ?? 'ssh-target';
const desktopHost = process.env.E2E_DESKTOP_HOST ?? 'desktop-target';
const webHost = process.env.E2E_WEB_HOST ?? 'web-target';
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
		page.getByRole('link', { name: 'Connect', exact: true }).click()
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
		page.getByRole('link', { name: 'Connect', exact: true }).click()
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
		page.getByRole('link', { name: 'Connect', exact: true }).click()
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
	// Searching shows the matches as a list, each with its host.
	const entry = bob
		.getByRole('list', { name: 'Search results' })
		.getByRole('button', { name: `${name} ${sshHost}` });
	await bob.getByRole('searchbox').fill(name);
	await entry.click();
	await expect(bob.getByRole('link', { name: 'Connect', exact: true })).toHaveCount(0);
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
	await expect(bob.getByRole('link', { name: 'Connect', exact: true })).toBeVisible();
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
		page.getByRole('link', { name: 'Connect', exact: true }).click()
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

test('the personal vault opens only in the browser, with passphrase, passkey or recovery key', async ({
	page,
	context
}) => {
	// A passkey that can derive secrets (WebAuthn PRF), as Chromium emulates it.
	const cdp = await context.newCDPSession(page);
	await cdp.send('WebAuthn.enable');
	await cdp.send('WebAuthn.addVirtualAuthenticator', {
		options: {
			protocol: 'ctap2',
			transport: 'internal',
			hasResidentKey: true,
			hasUserVerification: true,
			isUserVerified: true,
			automaticPresenceSimulation: true,
			hasPrf: true
		}
	});
	await signIn(page);
	await page.getByRole('link', { name: 'My vault' }).click();

	// Start from nothing: an earlier run may have left a vault behind.
	const unlockHeading = page.getByRole('heading', { name: 'Unlock your vault' });
	const setupHeading = page.getByRole('heading', { name: 'Set up your vault' });
	await expect(unlockHeading.or(setupHeading)).toBeVisible();
	if (await unlockHeading.isVisible()) {
		await page.getByRole('button', { name: 'Start over' }).click();
		await page.getByRole('dialog').getByRole('button', { name: 'Start over' }).click();
		await expect(setupHeading).toBeVisible();
	}
	const passphrase = 'long enough passphrase';
	await page.getByLabel('Passphrase', { exact: true }).fill(passphrase);
	await page.getByLabel('Passphrase again').fill(passphrase);
	await page.getByRole('button', { name: 'Set up' }).click();
	const recovery = (await page.getByTestId('recovery-key').innerText()).trim();
	expect(recovery).toMatch(/^([A-Z2-7]{4}-){7}[A-Z2-7]{4}$/);
	await page.getByRole('button', { name: 'I have kept it safe' }).click();

	const secret = `Router-Pw-${run}`;
	await page.getByRole('button', { name: 'New entry' }).click();
	const dialog = page.getByRole('dialog');
	await dialog.getByLabel('Title').fill('Office router');
	await dialog.getByLabel('User name').fill('admin');
	await dialog.getByLabel('Password').fill(secret);
	await dialog.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('Office router')).toBeVisible();
	await expect(page.getByText(secret)).toHaveCount(0);
	await page.getByRole('button', { name: 'Show' }).click();
	await expect(page.getByText(secret)).toBeVisible();

	// The server holds nothing readable of it.
	const stored = await page.evaluate(async () => {
		const vault = await (await fetch('/api/personal/vault')).json();
		return (
			JSON.stringify(vault) +
			vault.entries.map((e: { ciphertext: string }) => atob(e.ciphertext)).join('')
		);
	});
	expect(stored).not.toContain(secret);
	expect(stored).not.toContain('Office router');

	await page.getByLabel('Name of the passkey').fill('virtual key');
	await page.getByRole('button', { name: 'Add a passkey' }).click();
	await expect(page.getByText('Passkey · virtual key')).toBeVisible();

	const lock = () => page.getByRole('button', { name: 'Lock' }).click();
	await lock();
	await page.getByRole('button', { name: 'Unlock with a passkey' }).click();
	await expect(page.getByText('Office router')).toBeVisible();

	await lock();
	await page.getByLabel('Passphrase', { exact: true }).fill('not the passphrase');
	await page.getByRole('button', { name: 'Unlock', exact: true }).click();
	await expect(page.getByRole('alert')).toHaveText('That does not unlock the vault.');
	await page.getByRole('button', { name: 'Use the recovery key' }).click();
	await page.getByLabel('Recovery key').fill(recovery.toLowerCase());
	await page.getByRole('button', { name: 'Unlock', exact: true }).click();
	await expect(page.getByText('Office router')).toBeVisible();

	// A reload forgets the key; the passphrase opens it again.
	await page.reload();
	await expect(unlockHeading).toBeVisible();
	await page.getByLabel('Passphrase', { exact: true }).fill(passphrase);
	await page.getByRole('button', { name: 'Unlock', exact: true }).click();
	await expect(page.getByText('Office router')).toBeVisible();
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
		page.getByRole('link', { name: 'Connect', exact: true }).click()
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

test('the search puts first what was picked for the same query before', async ({ page }) => {
	await signIn(page);
	await newFolder(page, `pick one ${run}`);
	await newFolder(page, `pick two ${run}`);
	const search = page.getByRole('searchbox');
	const results = page.getByRole('list', { name: 'Search results' }).getByRole('button');

	await search.fill(`pick ${run}`);
	await expect(results.first()).toContainText(`pick one ${run}`);
	await results.filter({ hasText: `pick two ${run}` }).click();

	// Remembered on the server: a new page load still knows it.
	await page.reload();
	await search.fill(`pick ${run}`);
	await expect(results.first()).toContainText(`pick two ${run}`);
	// Enter takes the first result.
	await search.press('Enter');
	await expect(page.getByRole('heading', { name: `pick two ${run}`, level: 2 })).toBeVisible();
});

test('an administrator sets up a site connector and a device names it', async ({ page }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const site = `E2E site ${run}`;
	await page.getByRole('link', { name: 'Connectors' }).click();
	await page.getByRole('button', { name: 'New connector' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(site);
	await dialog.getByRole('button', { name: 'Create' }).click();
	// The token, once, with the address the connector needs.
	const settings = dialog.getByLabel('Settings for the connector');
	await expect(settings).toContainText('REMOTEHUB_CONNECTOR_TOKEN=rhc_');
	await expect(settings).toContainText('REMOTEHUB_URL=http');
	await dialog.getByRole('button', { name: 'Close', exact: true }).last().click();
	const row = page.getByRole('row', { name: new RegExp(site) });
	await expect(row).toContainText('not connected');

	// A device behind it shows where it is reached through.
	const folder = `E2E sites ${run}`;
	await page.getByRole('link', { name: 'Devices' }).click();
	await newFolder(page, folder);
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(`router ${run}`);
	await dialog.getByLabel('Host name or IP address').fill('10.20.0.1');
	await dialog.getByLabel('Reached through').selectOption({ label: site });
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: `router ${run}` })).toBeVisible();
	await expect(page.getByText(`${site} · not connected`)).toBeVisible();
});

test('a web interface opens signed in, in a browser on the server', async ({ page, context }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E appliances ${run}`;
	await newFolder(page, folder);

	const credential = `appliance tester ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Password', { exact: true }).fill('Tester-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();

	await newDevice(page, folder, `lab appliance ${run}`, credential, {
		label: 'Web interface (HTTPS)',
		host: webHost
	});
	const [appliance] = await Promise.all([
		context.waitForEvent('page'),
		page.getByRole('link', { name: 'Connect', exact: true }).click()
	]);
	await expect(appliance.getByText(/the certificate .* is now pinned/)).toBeVisible();
	await expect(appliance.getByRole('application')).toBeVisible();

	// The lab appliance turns green (#2e7d32) once signed in, red if not.
	await expect
		.poll(
			() =>
				appliance.evaluate(() => {
					const canvas = document.querySelector<HTMLCanvasElement>('[role=application] canvas');
					const pixel = canvas?.getContext('2d')?.getImageData(4, 4, 1, 1).data;
					return pixel ? [pixel[0], pixel[1], pixel[2]] : null;
				}),
			{ timeout: 15_000 }
		)
		.toEqual([46, 125, 50]);
	await appliance.close();
});
