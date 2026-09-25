import { expect, test, type Page } from '@playwright/test';
import { createHmac } from 'node:crypto';

// Local accounts through Ory Kratos (#103): an invited account sets its
// password and authenticator app, then signs in with both.

/** Kratos' admin API; the tests reach it on the compose network. */
const kratosAdmin = process.env.E2E_KRATOS_ADMIN_URL ?? 'http://kratos:4434';
const run = Date.now().toString(36);

/** What `remotehub account invite` does: an identity and its one-time code. */
async function invite(email: string, name: string) {
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
	return (await code.json()) as { recovery_link: string; recovery_code: string };
}

/** RFC 6238 with SHA-1 and six digits, as authenticator apps compute it. */
function totp(secret: string, step: number): string {
	const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
	let bits = '';
	for (const char of secret.replace(/=+$/, '').toUpperCase()) {
		bits += alphabet.indexOf(char).toString(2).padStart(5, '0');
	}
	const key = Buffer.from(bits.match(/.{8}/g)!.map((byte) => parseInt(byte, 2)));
	const counter = Buffer.alloc(8);
	counter.writeBigUInt64BE(BigInt(step));
	const hmac = createHmac('sha1', key).update(counter).digest();
	const offset = hmac[hmac.length - 1] & 0xf;
	const value = (hmac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000;
	return value.toString().padStart(6, '0');
}

const step = () => Math.floor(Date.now() / 30_000);

/** An address no other test, and no repetition, uses. */
const address = (who: string) => `${who}-${run}-${crypto.randomUUID().slice(0, 8)}@remotehub.test`;

/**
 * Invites an account and walks through the invitation: code, password,
 * authenticator app. Returns the app's key; the account is signed in.
 */
async function onboard(page: Page, email: string, password: string, name = '') {
	const invitation = await invite(email, name);
	return accept(page, invitation.recovery_link, invitation.recovery_code, password);
}

/** Walks through an invitation's link and code; see `onboard`. */
async function accept(page: Page, recoveryLink: string, code: string, password: string) {
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

/** Signs in with a directory account of the lab, up to the password. */
async function typeDirectory(page: Page, user: string, password: string) {
	await page.goto('/sign-in');
	await page.getByLabel('User name or e-mail').fill(user);
	await page.getByLabel('Password').fill(password);
	await page.getByRole('button', { name: 'Sign in' }).click();
}

async function signOut(page: Page) {
	await page.getByRole('button', { name: 'Sign out' }).click();
	await expect(page).toHaveURL(/\/sign-in$/);
}

async function signInLocal(page: Page, email: string, password: string, secret: string) {
	await page.goto('/sign-in');
	await page.getByLabel('User name or e-mail').fill(email);
	await page.getByLabel('Password').fill(password);
	await page.getByRole('button', { name: 'Sign in' }).click();
	// The next time step: another code than the one used just before, and
	// still within what Kratos accepts.
	await page.getByLabel('Code', { exact: true }).fill(totp(secret, step() + 1));
	await page.getByRole('button', { name: 'Confirm' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
}

test('an invited account sets its password and second factor, then signs in with both', async ({
	page
}) => {
	const email = address('local');
	const password = `Local-Passw0rd-${run}`;
	const secret = await onboard(page, email, password, 'Lena Local');
	await expect(page.getByRole('link', { name: /Lena Local/ })).toBeVisible();

	// Signed out, the password alone is not enough.
	await signOut(page);
	await signInLocal(page, email, password, secret);

	await page.getByRole('link', { name: /Lena Local/ }).click();
	await expect(page.getByRole('heading', { name: 'My account' })).toBeVisible();
	await expect(page.getByText('Set up. Every sign-in asks for its code.')).toBeVisible();
});

test('a wrong code keeps an account with a second factor out', async ({ page }) => {
	const email = address('wrong');
	const password = `Wrong-Passw0rd-${run}`;
	const secret = await onboard(page, email, password);
	await signOut(page);

	await page.goto('/sign-in');
	await page.getByLabel('User name or e-mail').fill(email);
	await page.getByLabel('Password').fill(password);
	await page.getByRole('button', { name: 'Sign in' }).click();
	const wrong = totp(secret, step() + 5);
	await page.getByLabel('Code', { exact: true }).fill(wrong);
	await page.getByRole('button', { name: 'Confirm' }).click();
	await expect(page.getByRole('alert')).toHaveText('The code is not right. Please try again.');
	// No remotehub session came of it.
	const me = await page.evaluate(async () => (await fetch('/api/session')).status);
	expect(me).toBe(401);
});

test('an administrator invites an account on the users page and blocks it', async ({
	page,
	browser
}) => {
	// alice administers through the directory.
	await typeDirectory(page, 'alice', 'Alice-Passw0rd!');
	await page.getByRole('link', { name: 'Users' }).click();

	const email = address('invited');
	const name = `Ines ${email.split('@')[0]}`;
	const dialog = page.getByRole('dialog');
	await page.getByRole('button', { name: 'Invite', exact: true }).click();
	await dialog.getByLabel('E-mail').fill(email);
	await dialog.getByLabel('Name').fill(name);
	await dialog.getByRole('button', { name: 'Invite', exact: true }).click();
	const link = (await dialog.getByTestId('code-link').innerText()).trim();
	const code = (await dialog.getByTestId('code-value').innerText()).trim();
	await dialog.getByRole('button', { name: 'Close' }).last().click();

	// Ines, in a browser of her own.
	const own = await browser.newContext();
	const ines = await own.newPage();
	await accept(ines, link, code, `Invited-Passw0rd-${run}`);

	await page.reload();
	const row = page.getByTestId('user-row').filter({ hasText: email });
	await expect(row).toContainText('Local');
	await expect(row).toContainText('Active');
	await row.getByRole('button', { name: `Actions for ${name}` }).click();
	await page.getByRole('menuitem', { name: 'Block' }).click();
	await dialog.getByRole('button', { name: 'Block' }).click();
	await expect(row).toContainText('Blocked');

	// Her session is gone: the next request sends her to the sign-in.
	await ines.reload();
	await expect(ines).toHaveURL(/\/sign-in$/);
	await own.close();
});

test('one form signs in directory accounts by name and by principal name', async ({ page }) => {
	// An address that is no local account goes on to the directory.
	await typeDirectory(page, 'bob@remotehub.test', 'Bob-Passw0rd!');
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
	await signOut(page);

	// A wrong password is refused by both, with one message.
	await typeDirectory(page, 'bob@remotehub.test', 'wrong');
	await expect(page.getByRole('alert')).toHaveCount(1);
	await expect(page).toHaveURL(/\/sign-in$/);
});

test('a directory account that needs a second factor sets it up at sign-in', async ({
	page,
	browser
}) => {
	// alice asks dave for an app; an earlier run may have left one.
	const admin = await (await browser.newContext()).newPage();
	await typeDirectory(admin, 'alice', 'Alice-Passw0rd!');
	await expect(admin.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
	await admin.evaluate(async () => {
		const found = await (await fetch('/api/directory/principals?q=dave')).json();
		const dave = found.find((p: { name: string }) => p.name.startsWith('Dave'));
		await fetch(`/api/second-factor-principals/${dave.sid}`, {
			method: 'PUT',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ principal_kind: 'user', principal_name: dave.name })
		});
		const users = await (await fetch('/api/users')).json();
		const known = users.find((u: { username: string }) => u.username === 'dave');
		if (known) await fetch(`/api/users/${known.id}/second-factor`, { method: 'DELETE' });
	});

	await typeDirectory(page, 'dave', 'Dave-Passw0rd!');
	await expect(page.getByText('Your account needs an authenticator app.')).toBeVisible();
	const secret = (await page.getByTestId('totp-secret').innerText()).trim();
	await page.getByLabel('Code from the app').fill(totp(secret, step()));
	await page.getByRole('button', { name: 'Set up' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();

	// From now on, every sign-in asks for a code.
	await signOut(page);
	await typeDirectory(page, 'dave', 'Dave-Passw0rd!');
	await page.getByLabel('Code', { exact: true }).fill(totp(secret, step() + 5));
	await page.getByRole('button', { name: 'Confirm' }).click();
	await expect(page.getByRole('alert')).toHaveText('The code is not right. Please try again.');
	await page.getByLabel('Code', { exact: true }).fill(totp(secret, step() + 1));
	await page.getByRole('button', { name: 'Confirm' }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();

	await admin.evaluate(async () => {
		const rules = await (await fetch('/api/second-factor-principals')).json();
		for (const rule of rules) {
			await fetch(`/api/second-factor-principals/${rule.principal_sid}`, { method: 'DELETE' });
		}
	});
});

test('a Kratos session left in the browser never signs in whoever types', async ({
	page,
	context
}) => {
	const first = address('first');
	const second = address('second');
	const password = `Shared-Passw0rd-${run}`;
	const firstSecret = await onboard(page, first, password, 'First Person');
	await signOut(page);
	const secret = await onboard(page, second, password, 'Second Person');
	await signOut(page);

	// The first signs in and leaves without signing out: the browser keeps
	// Kratos' session, only remotehub's goes (it ends, or its cookie is gone).
	await signInLocal(page, first, password, firstSecret);
	await context.clearCookies({ name: '__Host-remotehub-session' });

	// The second types their own credentials and becomes the second.
	await signInLocal(page, second, password, secret);
	await expect(page.getByRole('link', { name: /Second Person/ })).toBeVisible();
});
