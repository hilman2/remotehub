import { expect, test, type Download, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { readKdbx } from '../../src/lib/vault/kdbx';

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

/**
 * Sets up the signed-in user's personal vault anew with `passphrase`: an
 * earlier run may have left one behind. Returns the recovery key.
 */
async function freshVault(page: Page, passphrase: string) {
	await page.getByRole('link', { name: 'My vault' }).click();
	const unlockHeading = page.getByRole('heading', { name: 'Unlock your vault' });
	const setupHeading = page.getByRole('heading', { name: 'Set up your vault' });
	await expect(unlockHeading.or(setupHeading)).toBeVisible();
	if (await unlockHeading.isVisible()) {
		await page.getByRole('button', { name: 'Start over' }).click();
		await page.getByRole('dialog').getByRole('button', { name: 'Start over' }).click();
		await expect(setupHeading).toBeVisible();
	}
	await page.getByLabel('Passphrase', { exact: true }).fill(passphrase);
	await page.getByLabel('Passphrase again').fill(passphrase);
	await page.getByRole('button', { name: 'Set up' }).click();
	const recovery = (await page.getByTestId('recovery-key').innerText()).trim();
	await page.getByRole('button', { name: 'I have kept it safe' }).click();
	return recovery;
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

test('an AD user adds an SSH device and works in its terminal', async ({ page }) => {
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
	const name = `lab ssh ${run}`;
	await newDevice(page, folder, name, credential);

	// Its terminal opens as a tab inside remotehub; the first connection pins
	// the host key.
	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(page.getByText(/is now pinned/)).toBeVisible();
	await page.locator('.xterm').click();
	await page.keyboard.type('echo "e2e says $(whoami)"');
	await page.keyboard.press('Enter');
	await expect(page.locator('.xterm-rows')).toContainText('e2e says tester');

	// The connection is in the audit log.
	await page.getByRole('link', { name: 'Audit log' }).click();
	await expect(page.getByText('Opened a connection').first()).toBeVisible();

	// The session ran on meanwhile: its tab brings back the same terminal.
	const sessions = page.getByRole('navigation', { name: 'Open sessions' });
	await sessions
		.getByRole('button', { name: new RegExp(name) })
		.first()
		.click();
	await expect(page.locator('.xterm-rows')).toContainText('e2e says tester');

	// The devices wait in a strip; closing the tab ends the session.
	await page.getByRole('button', { name: 'Show devices' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
	await expect(page.locator('.xterm')).toBeHidden();
	await sessions.getByRole('button', { name: `Close ${name}` }).click();
	await expect(sessions).toHaveCount(0);
	await expect(page.locator('.xterm')).toHaveCount(0);
});

test('a stored password is shown and copied, and the audit log knows', async ({
	page,
	context
}) => {
	await context.grantPermissions(['clipboard-read', 'clipboard-write']);
	await signIn(page);
	const dialog = page.getByRole('dialog');
	await newFolder(page, `E2E reveal ${run}`);
	const credential = `reveal ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Password', { exact: true }).fill('Shown-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();
	await expect(page.locator('body')).not.toContainText('Shown-Passw0rd!');

	await page.getByRole('button', { name: 'Show', exact: true }).click();
	await expect(page.getByTestId('revealed')).toHaveText('Shown-Passw0rd!');
	await page.getByRole('button', { name: 'Hide', exact: true }).click();
	await expect(page.getByTestId('revealed')).toHaveCount(0);

	await page.getByRole('button', { name: 'Copy', exact: true }).click();
	await expect(page.getByRole('status').filter({ hasText: 'Copied' })).toBeVisible();
	expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('Shown-Passw0rd!');

	// The newest two entries, whatever an earlier run left.
	await page.getByRole('link', { name: 'Audit log' }).click();
	const rows = page.getByRole('row');
	for (const row of [rows.nth(1), rows.nth(2)]) {
		await expect(row).toContainText('Showed or copied a stored credential');
	}
});

test('the vault keeps folders, fields and icons, and shows what is shared', async ({ page }) => {
	await signIn(page);
	// A shared credential with a protected field, as the devices page makes it.
	const shared = `Shared router ${run}`;
	await page.evaluate(
		async ({ shared }) => {
			const post = (uri: string, body: unknown) =>
				fetch(uri, {
					method: 'POST',
					headers: { 'content-type': 'application/json' },
					body: JSON.stringify(body)
				}).then((r) => r.json());
			const folder = await post('/api/folders', { parent_id: null, name: `${shared} folder` });
			await post('/api/credentials', {
				folder_id: folder.id,
				name: shared,
				username: 'admin',
				password: 'Shared-Pass!',
				url: 'https://router.lan',
				icon: 3,
				fields: [{ name: 'PUK', protected: true, value: '8765' }]
			});
		},
		{ shared }
	);

	await freshVault(page, 'long enough passphrase');
	const dialog = page.getByRole('dialog');
	await page.getByRole('button', { name: 'New folder' }).click();
	await dialog.getByLabel('Name').fill('Bank');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await page.getByRole('button', { name: 'Bank', exact: true }).click();

	await page.getByRole('button', { name: 'New entry' }).click();
	await dialog.getByLabel('Title').fill('Online banking');
	await dialog.getByLabel('Password').fill('Bank-Pass!');
	await dialog.getByRole('button', { name: 'Add field' }).click();
	await dialog.getByLabel('Field name').fill('PIN');
	await dialog.getByLabel('Value').fill('1234');
	await dialog.getByLabel('Protected').check();
	await dialog.getByRole('radio', { name: 'Icon 37' }).click();
	await dialog.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('Online banking')).toBeVisible();
	await expect(page.getByText('PIN: ••••••')).toBeVisible();
	await page.getByRole('button', { name: 'Show', exact: true }).click();
	await expect(page.getByText('1234')).toBeVisible();

	// It lies in its folder: the top does not show it.
	await page
		.getByRole('navigation', { name: 'Folders' })
		.getByRole('button', { name: 'My vault' })
		.click();
	await expect(page.getByText('Online banking')).toHaveCount(0);

	// The shared credential, with its protected field on request.
	const entry = page.getByTestId('shared-entry').filter({ hasText: shared });
	await entry.getByRole('button', { name: new RegExp(shared) }).click();
	await expect(entry).toContainText('https://router.lan');
	await entry.getByRole('button', { name: 'Show', exact: true }).click();
	await expect(entry.getByTestId('revealed-field')).toHaveText('8765');
});

test('a shared credential keeps files and its earlier passwords', async ({ page }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	await newFolder(page, `E2E files ${run}`);
	const credential = `files ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('Password', { exact: true }).fill('Old-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();

	await page.locator('input[type=file]').setInputFiles({
		name: 'vpn.ovpn',
		mimeType: 'text/plain',
		buffer: Buffer.from('remote vpn.example.com')
	});
	const downloading = page.waitForEvent('download');
	await page.getByRole('link', { name: 'vpn.ovpn' }).click();
	const download = await downloading;
	expect(download.suggestedFilename()).toBe('vpn.ovpn');

	await page.getByRole('button', { name: 'Settings' }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	await dialog.getByLabel('Password', { exact: true }).fill('New-Passw0rd!');
	await dialog.getByRole('button', { name: 'Save' }).click();
	await page.getByRole('button', { name: 'Earlier versions' }).click();
	await page
		.getByRole('listitem')
		.filter({ hasText: 'Version 1' })
		.getByRole('button', { name: 'Show' })
		.click();
	await expect(page.getByTestId('earlier-password')).toHaveText('Old-Passw0rd!');
});

test('vault entries keep files, earlier passwords and show TOTP codes', async ({ page }) => {
	await signIn(page);
	await freshVault(page, 'long enough passphrase');
	const dialog = page.getByRole('dialog');
	await page.getByRole('button', { name: 'New entry' }).click();
	await dialog.getByLabel('Title').fill('Mail');
	await dialog.getByLabel('Password').fill('First-Pass!');
	await dialog.getByRole('button', { name: 'Add field' }).click();
	await dialog.getByLabel('Field name').fill('otp');
	await dialog
		.getByLabel('Value')
		.fill('otpauth://totp/mail?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ');
	await dialog.locator('input[type=file]').setInputFiles({
		name: 'backup-codes.txt',
		mimeType: 'text/plain',
		buffer: Buffer.from('11111 22222')
	});
	await dialog.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText('Mail', { exact: true })).toBeVisible();

	// A new password from the generator; the first one goes into the history.
	await page.getByRole('button', { name: 'Edit' }).click();
	await dialog.getByRole('button', { name: 'Generate a password' }).click();
	const generated = await dialog.getByLabel('Password').inputValue();
	expect(generated).toHaveLength(20);
	await dialog.getByRole('button', { name: 'Save' }).click();
	await page.getByRole('button', { name: 'Show', exact: true }).click();
	await expect(page.getByText(generated)).toBeVisible();
	await expect(page.getByTestId('totp-code')).toHaveText(/^\d{6}$/);
	await page.getByText('Earlier versions').click();
	await expect(page.getByTestId('earlier-password')).toHaveText('First-Pass!');

	// The file comes back as it went in, opened in the browser.
	const downloading = page.waitForEvent('download');
	await page.getByRole('button', { name: /backup-codes\.txt/ }).click();
	const download = await downloading;
	const chunks: Buffer[] = [];
	for await (const chunk of await download.createReadStream()) chunks.push(chunk as Buffer);
	expect(Buffer.concat(chunks).toString()).toBe('11111 22222');
});

test('an SSH key protected by a passphrase signs in', async ({ page }) => {
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
	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	// Typing waits for the session, as a person would.
	await expect(page.getByText(/host key/)).toBeVisible();
	await page.locator('.xterm').click();
	await page.keyboard.type('echo "key says $(whoami)"');
	await page.keyboard.press('Enter');
	await expect(page.locator('.xterm-rows')).toContainText('key says tester');
});

test('an SSH device signs in with a certificate from remotehub', async ({ page }) => {
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

	// A double-click in the tree connects as well.
	await page
		.getByRole('tree')
		.getByRole('button', { name: new RegExp(name) })
		.dblclick();
	await expect(page.getByText(/host key/)).toBeVisible();
	await page.locator('.xterm').click();
	await page.keyboard.type('echo "ca says $(whoami)"');
	await page.keyboard.press('Enter');
	await expect(page.locator('.xterm-rows')).toContainText('ca says alice');
});

test('a device signs in with credentials of its own', async ({ page }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E own ${run}`;
	await newFolder(page, folder);

	// No credential object: user name and password belong to the device.
	const name = `lab ssh own ${run}`;
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog.getByLabel('Sign in with').selectOption({ label: 'Credentials of this device' });
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Password', { exact: true }).fill('Tester-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();
	await expect(page.locator('body')).not.toContainText('Tester-Passw0rd!');

	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(page.getByText(/host key/)).toBeVisible();
	await page.locator('.xterm').click();
	await page.keyboard.type('echo "own says $(whoami)"');
	await page.keyboard.press('Enter');
	await expect(page.locator('.xterm-rows')).toContainText('own says tester');

	// An SSH key of its own, protected by a passphrase, loaded from its file.
	await page.getByRole('button', { name: 'Show devices' }).click();
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	const keyed = `lab ssh own key ${run}`;
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(keyed);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog.getByLabel('Sign in with').selectOption({ label: 'Credentials of this device' });
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Type').selectOption({ label: 'SSH key' });
	await dialog
		.getByLabel('Load key from file')
		.setInputFiles(join(labKeys, 'tester_ed25519_passphrase'));
	await dialog.getByLabel('Passphrase').fill('Key-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: keyed })).toBeVisible();
	await expect(page.getByText(/SHA256:/).first()).toBeVisible();
	await expect(page.locator('body')).not.toContainText('PRIVATE KEY');

	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(page.getByText(/host key/)).toBeVisible();
	await page.locator('.xterm:visible').click();
	await page.keyboard.type('echo "own key says $(whoami)"');
	await page.keyboard.press('Enter');
	await expect(page.locator('.xterm-rows:visible')).toContainText('own key says tester');
});

test('a group of remotehub’s own passes a permission on to its members', async ({
	page,
	browser
}) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const group = `E2E team ${run}`;
	await page.getByRole('link', { name: 'Users' }).click();
	await page.getByRole('button', { name: 'New group' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(group);
	await dialog.getByRole('button', { name: 'Create' }).click();
	const row = page.getByTestId('group-row').filter({ hasText: group });
	await row.getByRole('button', { name: `Actions for ${group}` }).click();
	await page.getByRole('menuitem', { name: 'Members' }).click();
	await dialog.getByLabel('Search users and groups').fill('Bob');
	await dialog.getByRole('button', { name: /Bob Helpdesk/ }).click();
	await dialog.getByRole('button', { name: 'Add', exact: true }).click();
	await expect(dialog.getByRole('listitem').filter({ hasText: 'Bob Helpdesk' })).toBeVisible();
	await page.keyboard.press('Escape');
	await expect(row).toContainText('Bob Helpdesk');

	// The group may see a new device.
	await page.getByRole('link', { name: 'Devices' }).click();
	const folder = `E2E team folder ${run}`;
	await newFolder(page, folder);
	const name = `lab ssh team ${run}`;
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog.getByRole('button', { name: 'Create' }).click();
	await page.getByRole('button', { name: 'Settings' }).click();
	await page.getByRole('menuitem', { name: 'Permissions' }).click();
	await dialog.getByLabel('Search users and groups').fill(group);
	await dialog.getByRole('button', { name: group }).click();
	await dialog.locator('select').selectOption({ label: 'See' });
	await dialog.getByRole('button', { name: 'Grant' }).click();
	await expect(dialog.getByText(group)).toBeVisible();

	// bob sees it through the group.
	const bobs = await browser.newContext();
	const bob = await bobs.newPage();
	await signIn(bob, 'bob', 'Bob-Passw0rd!');
	await bob.getByRole('searchbox').fill(name);
	await expect(
		bob
			.getByRole('list', { name: 'Search results' })
			.getByRole('button', { name: `${name} ${sshHost}` })
	).toBeVisible();
	await bobs.close();
});

test('an auditor reads the audit log and manages nothing', async ({ page, browser }) => {
	const bobs = await browser.newContext();
	const bob = await bobs.newPage();
	await signIn(bob, 'bob', 'Bob-Passw0rd!');
	await expect(bob.getByRole('link', { name: 'Audit log' })).toHaveCount(0);

	await signIn(page);
	await page.getByRole('link', { name: 'Users' }).click();
	const auditors = page.getByTestId('role-auditor');
	await auditors.getByRole('button', { name: 'Give the role Auditor' }).click();
	const dialog = page.getByRole('dialog');
	await dialog.getByLabel('Search users and groups').fill('Bob');
	await dialog.getByRole('button', { name: /Bob Helpdesk/ }).click();
	await dialog.getByRole('button', { name: 'Add', exact: true }).click();
	await expect(auditors).toContainText('Bob Helpdesk');

	// bob's open session has the role once the page loads again.
	await bob.reload();
	await bob.getByRole('link', { name: 'Audit log' }).click();
	await expect(bob.getByText('Gave a role').first()).toBeVisible();
	await expect(bob.getByRole('link', { name: 'Users' })).toHaveCount(0);

	await auditors.getByRole('button', { name: 'Remove Bob Helpdesk' }).click();
	await expect(auditors).not.toContainText('Bob Helpdesk');
	await bobs.close();
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
	await page.getByRole('button', { name: 'Settings' }).click();
	await page.getByRole('menuitem', { name: 'Permissions' }).click();
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
	await expect(bob.getByRole('button', { name: 'Connect', exact: true })).toHaveCount(0);
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
	await expect(bob.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
	await bob.getByRole('link', { name: 'Access requests' }).click();
	await expect(
		bob
			.getByRole('listitem')
			.filter({ hasText: name })
			.getByText(/Approved · until .* · by alice/)
	).toBeVisible();
	await bobs.close();
});

test('chosen users state a purpose, and the device journal keeps it', async ({ page, browser }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E journal ${run}`;
	await newFolder(page, folder);
	const credential = `journal tester ${run}`;
	await page.getByRole('button', { name: 'New credential' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(credential);
	await dialog.getByLabel('User name', { exact: true }).fill('tester');
	await dialog.getByLabel('Password', { exact: true }).fill('Tester-Passw0rd!');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name: credential })).toBeVisible();
	const name = `lab ssh journal ${run}`;
	await newDevice(page, folder, name, credential);

	// bob may connect to it.
	await page.getByRole('button', { name: 'Settings' }).click();
	await page.getByRole('menuitem', { name: 'Permissions' }).click();
	await dialog.getByLabel('Search users and groups').fill('Bob');
	await dialog.getByRole('button', { name: /Bob Helpdesk/ }).click();
	await dialog.locator('select').selectOption({ label: 'Connect' });
	await dialog.getByRole('button', { name: 'Grant' }).click();
	await expect(dialog.getByText('Bob Helpdesk')).toBeVisible();
	await page.keyboard.press('Escape');

	// alice leaves a note in the device's journal.
	const journal = page.getByRole('list', { name: 'Journal' });
	// Ctrl+Enter saves it.
	await page.getByLabel('Note', { exact: true }).fill('Replaced the disk.');
	await page.getByLabel('Note', { exact: true }).press('Control+Enter');
	await expect(journal).toContainText('Replaced the disk.');

	// From now on bob states a purpose before connecting.
	await page.getByRole('link', { name: 'Settings' }).click();
	const section = page.getByRole('region', { name: 'Purpose before connecting' });
	const rules = section.getByRole('list', { name: 'Purpose before connecting' });
	await section.getByLabel('Search users and groups').fill('Bob');
	await section.getByRole('button', { name: /Bob Helpdesk/ }).click();
	await section.getByRole('button', { name: 'Add', exact: true }).click();
	await expect(rules).toContainText('Bob Helpdesk');

	const bobs = await browser.newContext();
	const bob = await bobs.newPage();
	await signIn(bob, 'bob', 'Bob-Passw0rd!');
	await bob.getByRole('searchbox').fill(name);
	await bob
		.getByRole('list', { name: 'Search results' })
		.getByRole('button', { name: new RegExp(name) })
		.click();
	await bob.getByRole('button', { name: 'Connect', exact: true }).click();
	await bob.getByLabel('Purpose').fill('Rotate the logs');
	await bob.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(bob.getByText(/host key/)).toBeVisible();

	// The journal says who connected and why, next to alice's note.
	await bob.getByRole('button', { name: 'Show devices' }).click();
	const bobsJournal = bob.getByRole('list', { name: 'Journal' });
	await expect(bobsJournal.getByRole('listitem').first()).toContainText('Rotate the logs');
	await expect(bobsJournal.getByRole('listitem').first()).toContainText('Bob Helpdesk');
	await expect(bobsJournal).toContainText('Replaced the disk.');
	await bobs.close();

	// The development database outlives the test: bob goes off the list again.
	await rules
		.getByRole('listitem')
		.filter({ hasText: 'Bob Helpdesk' })
		.getByRole('button', { name: 'Remove' })
		.click();
	await expect(page.getByText('Bob Helpdesk')).toHaveCount(0);
});

test('an RDP desktop opens with the password LAPS keeps in the directory', async ({ page }) => {
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

	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(page.getByRole('application')).toBeVisible();
	// The lab desktop's background (#1e5b8c): the sign-in worked.
	await expect
		.poll(
			() =>
				page.evaluate(() => {
					const canvas = document.querySelector<HTMLCanvasElement>('[role=application] canvas');
					const pixel = canvas?.getContext('2d')?.getImageData(4, 4, 1, 1).data;
					return pixel ? [pixel[0], pixel[1], pixel[2]] : null;
				}),
			{ timeout: 15_000 }
		)
		.toEqual([30, 91, 140]);
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
	const passphrase = 'long enough passphrase';
	const recovery = await freshVault(page, passphrase);
	expect(recovery).toMatch(/^([A-Z2-7]{4}-){7}[A-Z2-7]{4}$/);
	const unlockHeading = page.getByRole('heading', { name: 'Unlock your vault' });

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

test('a device that asks for credentials takes them from the unlocked vault', async ({ page }) => {
	await signIn(page);
	await freshVault(page, 'long enough passphrase');
	const dialog = page.getByRole('dialog');
	await page.getByRole('button', { name: 'New entry' }).click();
	await dialog.getByLabel('Title').fill(`Lab tester ${run}`);
	await dialog.getByLabel('User name').fill('tester');
	await dialog.getByLabel('Password').fill('Tester-Passw0rd!');
	await dialog.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText(`Lab tester ${run}`)).toBeVisible();

	// The vault stays open on the way to the devices.
	await page.getByRole('link', { name: 'Devices' }).click();
	await newFolder(page, `E2E from vault ${run}`);
	const name = `lab ssh asks ${run}`;
	await page.getByRole('button', { name: 'New device' }).click();
	await dialog.getByLabel('Name', { exact: true }).fill(name);
	await dialog.getByLabel('Host name or IP address').fill(sshHost);
	await dialog
		.getByLabel('Sign in with')
		.selectOption({ label: 'Credentials asked for when connecting' });
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(page.getByRole('heading', { name })).toBeVisible();

	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await page.getByLabel('From my vault').selectOption({ label: `Lab tester ${run} · tester` });
	await expect(page.getByLabel('User name')).toHaveValue('tester');
	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(page.getByText(/host key/)).toBeVisible();
	await page.locator('.xterm').click();
	await page.keyboard.type('echo "vault says $(whoami)"');
	await page.keyboard.press('Enter');
	await expect(page.locator('.xterm-rows')).toContainText('vault says tester');
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
	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	// The first connection pins the device's certificate.
	await expect(page.getByText(/the certificate .* is now pinned/)).toBeVisible();
	await expect(page.getByRole('application')).toBeVisible();

	// The lab desktop's background (#1e5b8c) in the corner of the picture.
	await expect
		.poll(
			() =>
				page.evaluate(() => {
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
	const clipboard = () => page.evaluate(() => navigator.clipboard.readText());
	const copy = (text: string) => page.evaluate((text) => navigator.clipboard.writeText(text), text);
	await copy('copied in the browser');
	await page.getByRole('application').click();
	await expect.poll(clipboard).toBe('echo:copied in the browser');
	// …then what was copied while it had focus, with the paste key.
	await copy('pasted with Ctrl+V');
	await page.keyboard.press('Control+V');
	await expect.poll(clipboard).toBe('echo:pasted with Ctrl+V');
});

test('folders start closed and stay as the user left them', async ({ page }) => {
	await signIn(page);
	const parent = `outer ${run}`;
	const child = `inner ${run}`;
	await newFolder(page, parent);
	await page.getByRole('button', { name: 'New subfolder' }).click();
	await page.getByRole('dialog').getByLabel('Name', { exact: true }).fill(child);
	await page.getByRole('dialog').getByRole('button', { name: 'Create' }).click();

	const tree = page.getByRole('tree');
	const item = (name: string) => tree.getByRole('treeitem').filter({ hasText: name }).first();
	const inner = tree.getByRole('button', { name: child, exact: true });
	// A new subfolder opens its parent, so it is visible.
	await expect(inner).toBeVisible();

	await item(parent).getByRole('button', { name: 'Collapse' }).click();
	await expect(inner).toHaveCount(0);
	await page.reload();
	await expect(tree.getByRole('button', { name: parent, exact: true })).toBeVisible();
	await expect(inner).toHaveCount(0);

	await item(parent).getByRole('button', { name: 'Expand' }).click();
	await expect(inner).toBeVisible();
	await page.reload();
	await expect(inner).toBeVisible();
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

test('a web interface opens signed in, in a browser on the server', async ({ page }) => {
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
	await page.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(page.getByText(/the certificate .* is now pinned/)).toBeVisible();
	await expect(page.getByRole('application')).toBeVisible();

	// The lab appliance turns green (#2e7d32) once signed in, red if not.
	await expect
		.poll(
			() =>
				page.evaluate(() => {
					const canvas = document.querySelector<HTMLCanvasElement>('[role=application] canvas');
					const pixel = canvas?.getContext('2d')?.getImageData(4, 4, 1, 1).data;
					return pixel ? [pixel[0], pixel[1], pixel[2]] : null;
				}),
			{ timeout: 15_000 }
		)
		.toEqual([46, 125, 50]);
});

/** Made by KeePassXC: the group Servers with the entry Router and its file vpn.txt. */
const keepassFixture = join(process.cwd(), 'src', 'lib', 'vault', 'fixtures', 'keepassxc.kdbx');

/** Opens the KeePass file a download hands over. */
async function openDownload(downloading: Promise<Download>, password: string) {
	const file = readFileSync(await (await downloading).path());
	return readKdbx(file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength), password);
}

test('the vault takes in a KeePass file and gives one back', async ({ page }) => {
	await signIn(page);
	await freshVault(page, 'long enough passphrase');
	const dialog = page.getByRole('dialog');
	await page.getByRole('button', { name: 'Import a KeePass file' }).click();
	await dialog.getByLabel('KeePass file').setInputFiles(keepassFixture);
	await dialog.getByLabel('Its master password').fill('Fixture-Passw0rd');
	await dialog.getByRole('button', { name: 'Import a KeePass file' }).click();
	await expect(dialog.getByRole('status')).toHaveText('Entries imported: 1.');
	await page.keyboard.press('Escape');
	await page.getByRole('button', { name: 'Servers', exact: true }).click();
	await expect(page.getByText('admin · https://router.lan')).toBeVisible();

	await page.getByRole('button', { name: 'Export as a KeePass file' }).click();
	await dialog.getByLabel('Master password of the new file').fill('Export-Passw0rd');
	await dialog.getByLabel('Passphrase again').fill('Export-Passw0rd');
	const downloading = page.waitForEvent('download');
	await dialog.getByRole('button', { name: 'Export as a KeePass file' }).click();
	const [router, ...rest] = await openDownload(downloading, 'Export-Passw0rd');
	expect(rest).toEqual([]);
	expect(router).toMatchObject({ path: ['Servers'], title: 'Router', password: 'Entry-Pass!' });
	expect(router.files.map((f) => new TextDecoder().decode(f.data))).toEqual(['remote vpn']);
});

test('a shared folder takes in a KeePass file and exports it, audited', async ({ page }) => {
	await signIn(page);
	const dialog = page.getByRole('dialog');
	const folder = `E2E keepass ${run}`;
	await newFolder(page, folder);
	await page.getByRole('button', { name: 'Settings' }).click();
	await page.getByRole('menuitem', { name: 'Import a KeePass file' }).click();
	await dialog.getByLabel('KeePass file').setInputFiles(keepassFixture);
	await dialog.getByLabel('Its master password').fill('Fixture-Passw0rd');
	await dialog.getByRole('button', { name: 'Import a KeePass file' }).click();
	await expect(dialog.getByRole('status')).toHaveText('Entries imported: 1.');
	await page.keyboard.press('Escape');

	// The export reads back the group as a folder below the one exported.
	await page.getByRole('tree').getByRole('button', { name: folder, exact: true }).click();
	await page.getByRole('button', { name: 'Settings' }).click();
	await page.getByRole('menuitem', { name: 'Export as a KeePass file' }).click();
	await dialog.getByLabel('Master password of the new file').fill('Export-Passw0rd');
	await dialog.getByLabel('Passphrase again').fill('Export-Passw0rd');
	const downloading = page.waitForEvent('download');
	await dialog.getByRole('button', { name: 'Export as a KeePass file' }).click();
	const [router, ...rest] = await openDownload(downloading, 'Export-Passw0rd');
	expect(rest).toEqual([]);
	expect(router).toMatchObject({ path: ['Servers'], title: 'Router', password: 'Entry-Pass!' });
	expect(router.files.map((f) => new TextDecoder().decode(f.data))).toEqual(['remote vpn']);
	await page.keyboard.press('Escape');

	await page.getByRole('link', { name: 'Audit log' }).click();
	await expect(page.getByRole('row').nth(1)).toContainText('Showed or copied a stored credential');
});
