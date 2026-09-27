import {
	chromium,
	expect,
	test,
	type BrowserContext,
	type Page,
	type Worker
} from '@playwright/test';
import { join } from 'node:path';
import { build } from 'vite';

// The browser extension end to end (#201): Chromium loads the build
// unpacked, connects it through remotehub's page, and fills logins into
// pages that Playwright serves under made-up addresses. bob uses it, so his
// personal vault is his own: other files reset alice's.

const run = Date.now().toString(36);
const base = process.env.E2E_BASE_URL ?? 'http://localhost:5180';
/** The ID of the development key, which the development stack and the CI allow. */
const DEV_ID = 'obnekonmlefgdhgodgbjapgoophnhlao';
const built = join(process.cwd(), 'build-extension-e2e');

const LOGIN = `<h1>Portal</h1><form>
	<label>User <input name="user" autocomplete="username"></label>
	<label>Password <input name="pass" type="password" autocomplete="current-password"></label>
	<button>Sign in</button></form>`;
/** Pages of made-up sites, by address. */
const PAGES: Record<string, string> = {
	'https://portal.example.test/': LOGIN,
	'https://login.example.test.evil.test/': LOGIN,
	'https://evil.test/form': LOGIN,
	'https://portal.example.test/framed': `<h1>Framed</h1><iframe src="https://evil.test/form" title="Other site"></iframe>`,
	'https://portal.example.test/hidden': `<h1>Hidden</h1><form>
		<label>User <input name="username"></label>
		<label>Password <input name="pw" type="password" style="opacity: 0"></label></form>`,
	'https://portal.example.test/next': `<h1>Step one</h1><form>
		<label>Email <input name="email" type="email"></label><button>Next</button></form>`,
	'https://portal.example.test/code': `<h1>Code</h1><form>
		<label>Code <input name="otp" autocomplete="one-time-code"></label></form>`,
	'https://personal.example.test/': LOGIN
};

test.describe.configure({ mode: 'serial' });

let context: BrowserContext;
let worker: Worker;
let bob: Page;

test.beforeAll(async () => {
	process.env.REMOTEHUB_EXTENSION_E2E = '1';
	await build({ configFile: join(process.cwd(), 'vite.extension.config.ts'), logLevel: 'warn' });
	context = await chromium.launchPersistentContext('', {
		// Chromium's own build runs extensions without a window.
		channel: 'chromium',
		baseURL: base,
		locale: 'en-GB',
		args: [`--disable-extensions-except=${built}`, `--load-extension=${built}`]
	});
	await context.route(/^https:\/\/[^/]*\.?(example\.test|evil\.test)\//, (route) => {
		const body = PAGES[route.request().url()];
		return body
			? route.fulfill({ contentType: 'text/html', body: `<!doctype html>${body}` })
			: route.fulfill({ status: 404 });
	});
	worker = context.serviceWorkers()[0] ?? (await context.waitForEvent('serviceworker'));
	bob = await context.newPage();
});

test.afterAll(async () => {
	await context?.close();
});

async function signIn(page: Page, user: string, password: string) {
	await page.goto('/');
	await expect(page).toHaveURL(/\/sign-in$/);
	await page.getByLabel('User name').fill(user);
	await page.getByLabel('Password').fill(password);
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
}

/** A call to remotehub's API from a signed-in page; returns status and body. */
async function call(page: Page, method: string, path: string, body?: unknown) {
	return page.evaluate(
		async ([method, path, body]) => {
			const response = await fetch(path as string, {
				method: method as string,
				headers: { 'content-type': 'application/json' },
				body: body === undefined ? undefined : JSON.stringify(body)
			});
			return { status: response.status, body: await response.json().catch(() => null) };
		},
		[method, path, body]
	);
}

const extensionPage = (path: string) => `chrome-extension://${DEV_ID}/${path}`;

/** The popup, opened as a page for the tab showing `url`. */
async function popupFor(url: string): Promise<Page> {
	const tab = await worker.evaluate(
		async (url) => (await chrome.tabs.query({})).find((t) => t.url === url)?.id,
		url
	);
	expect(tab, url).toBeDefined();
	const popup = await context.newPage();
	await popup.goto(extensionPage(`popup.html?tab=${tab}`));
	return popup;
}

/** A new tab with `url`. */
async function open(url: string): Promise<Page> {
	const page = await context.newPage();
	await page.goto(url);
	return page;
}

const shared = `Portal ${run}`;
const coded = `Code ${run}`;

test('the extension connects through remotehub, as the extension remotehub knows', async ({
	page
}) => {
	// Chromium derives the ID from the key in the manifest as the packing does.
	expect(new URL(worker.url()).host).toBe(DEV_ID);

	// alice shares two logins with bob.
	await signIn(page, 'alice', 'Alice-Passw0rd!');
	const collection = await call(page, 'POST', '/api/collections', {
		parent_id: null,
		name: `Extension ${run}`
	});
	expect(collection.status).toBe(201);
	const id = collection.body.id;
	for (const [name, extra] of [
		[shared, {}],
		[coded, { totp: 'JBSWY3DPEHPK3PXP' }]
	] as const) {
		const created = await call(page, 'POST', '/api/credentials', {
			collection_id: id,
			name,
			username: 'portal-user',
			password: `Portal-Pw-${run}`,
			url: 'https://login.example.test',
			...extra
		});
		expect(created.status, JSON.stringify(created.body)).toBe(201);
	}
	const [principal] = (await call(page, 'GET', '/api/directory/principals?q=bob')).body;
	const granted = await call(page, 'POST', '/api/grants', {
		object: { kind: 'collection', id },
		principal_kind: 'user',
		principal_sid: principal.sid,
		principal_name: principal.name,
		role: 'reveal'
	});
	expect(granted.status).toBe(204);

	// bob connects the extension from its settings; remotehub's page asks.
	await signIn(bob, 'bob', 'Bob-Passw0rd!');
	const options = await open(extensionPage('options.html'));
	await options.getByLabel('Address of your remotehub').fill(base);
	const window = context.waitForEvent('page');
	await options.getByRole('button', { name: 'Connect' }).click();
	const connect = await window;
	await connect.waitForURL(/\/(extension\/connect|sign-in)/);
	if (connect.url().includes('/sign-in')) {
		// The sign-in window keeps its own cookies: bob signs in there and
		// comes back to the request.
		await connect.getByLabel('User name').fill('bob');
		await connect.getByLabel('Password').fill('Bob-Passw0rd!');
		await connect.getByRole('button', { name: 'Sign in', exact: true }).click();
	}
	await expect(
		connect.getByRole('heading', { name: 'Connect the browser extension' })
	).toBeVisible();
	await expect(connect.getByText(DEV_ID)).toBeVisible();
	await connect.getByRole('button', { name: 'Connect', exact: true }).click();
	await expect(options.getByText(`Connected to ${base} as Bob Helpdesk.`)).toBeVisible();
});

test('a shared login is filled into its site, and the audit log knows', async ({ page }) => {
	const portal = await open('https://portal.example.test/');
	const popup = await popupFor('https://portal.example.test/');
	await expect(
		popup.getByRole('heading', { name: /This page: portal\.example\.test/ })
	).toBeVisible();
	await expect(popup.getByRole('button', { name: `Fill ${coded}` })).toBeVisible();
	await popup.getByRole('button', { name: `Fill ${shared}` }).click();
	await expect(portal.getByLabel('User')).toHaveValue('portal-user');
	await expect(portal.getByLabel('Password')).toHaveValue(`Portal-Pw-${run}`);

	// The one-time code goes into the field meant for it.
	const second = await open('https://portal.example.test/code');
	const codes = await popupFor('https://portal.example.test/code');
	await codes.getByRole('button', { name: `Fill the one-time code of ${coded}` }).click();
	await expect(second.getByLabel('Code')).toHaveValue(/^\d{6}$/);

	await signIn(page, 'alice', 'Alice-Passw0rd!');
	const audit = (await call(page, 'GET', '/api/audit?limit=100')).body as {
		action: string;
		actor_name: string;
		details: Record<string, unknown>;
	}[];
	const byBob = audit.filter((e) => e.actor_name === 'bob');
	expect(byBob.find((e) => e.action === 'credential.revealed')?.details).toMatchObject({
		purpose: 'fill',
		origin: 'https://portal.example.test',
		client: 'extension'
	});
	expect(byBob.find((e) => e.action === 'credential.code_shown')?.details).toMatchObject({
		purpose: 'fill',
		origin: 'https://portal.example.test'
	});
	expect(byBob.some((e) => e.action === 'extension.connected')).toBe(true);
});

test('a copied password leaves the clipboard after half a minute', async () => {
	test.setTimeout(120_000);
	const reader = await open('https://portal.example.test/');
	await context.grantPermissions(['clipboard-read', 'clipboard-write'], {
		origin: 'https://portal.example.test'
	});
	const popup = await popupFor('https://portal.example.test/');
	await popup.bringToFront();
	await popup.getByRole('button', { name: `Copy the password of ${shared}` }).click();
	await expect(popup.getByRole('status')).toHaveText(
		'Copied. It leaves the clipboard in 30 seconds.'
	);
	await reader.bringToFront();
	const clipboard = () => reader.evaluate(() => navigator.clipboard.readText());
	expect(await clipboard()).toBe(`Portal-Pw-${run}`);
	await expect.poll(clipboard, { timeout: 60_000, intervals: [2_000] }).toBe('');
});

test('nothing is filled into other sites, frames of other sites or hidden fields', async () => {
	// A look-alike address: the login is not even offered.
	await open('https://login.example.test.evil.test/');
	const lookalike = await popupFor('https://login.example.test.evil.test/');
	await expect(lookalike.getByText('No login belongs to this page.')).toBeVisible();
	await expect(lookalike.getByRole('button', { name: `Fill ${shared}` })).toHaveCount(0);

	// The login's own site, but the form sits in a frame of another site.
	const framed = await open('https://portal.example.test/framed');
	const inFrame = await popupFor('https://portal.example.test/framed');
	await inFrame.getByRole('button', { name: `Fill ${shared}` }).click();
	await expect(inFrame.getByRole('alert')).toHaveText('This page shows no sign-in form.');
	const frame = framed.frameLocator('iframe');
	await expect(frame.getByLabel('User')).toHaveValue('');
	await expect(frame.getByLabel('Password')).toHaveValue('');

	// A transparent password field gets nothing; the user name does.
	const hidden = await open('https://portal.example.test/hidden');
	const withHidden = await popupFor('https://portal.example.test/hidden');
	await withHidden.getByRole('button', { name: `Fill ${shared}` }).click();
	await expect(withHidden.getByRole('status')).toHaveText(
		'User name filled in. The password follows on the next page.'
	);
	await expect(hidden.getByLabel('User')).toHaveValue('portal-user');
	await expect(hidden.getByLabel('Password')).toHaveValue('');

	// The first step of a two-step sign-in takes the user name alone.
	const next = await open('https://portal.example.test/next');
	const first = await popupFor('https://portal.example.test/next');
	await first.getByRole('button', { name: `Fill ${shared}` }).click();
	await expect(next.getByLabel('Email')).toHaveValue('portal-user');
});

test('personal logins open in the extension with the passphrase', async () => {
	// bob's personal vault, set up anew, with a login for a site.
	await bob.goto('/vault');
	await bob.getByRole('button', { name: /^All personal entries/ }).click();
	const unlockHeading = bob.getByRole('heading', { name: 'Unlock your vault' });
	const setupHeading = bob.getByRole('heading', { name: 'Set up your vault' });
	await expect(unlockHeading.or(setupHeading)).toBeVisible();
	if (await unlockHeading.isVisible()) {
		await bob.getByRole('button', { name: 'Start over' }).click();
		await bob.getByRole('dialog').getByRole('button', { name: 'Start over' }).click();
		await expect(setupHeading).toBeVisible();
	}
	const passphrase = `bob's passphrase ${run}`;
	await bob.getByLabel('Passphrase', { exact: true }).fill(passphrase);
	await bob.getByLabel('Passphrase again').fill(passphrase);
	await bob.getByRole('button', { name: 'Set up' }).click();
	await bob.getByRole('button', { name: 'I have kept it safe' }).click();
	await bob.getByRole('button', { name: 'New entry' }).click();
	const dialog = bob.getByRole('dialog');
	await dialog.getByLabel('Title').fill(`Mine ${run}`);
	await dialog.getByLabel('User name').fill('bob-himself');
	await dialog.getByLabel('Password').fill(`Mine-Pw-${run}`);
	await dialog.getByLabel('Address', { exact: true }).fill('https://personal.example.test');
	await dialog.getByRole('button', { name: 'Create' }).click();
	await expect(bob.getByTestId('personal-entry').filter({ hasText: `Mine ${run}` })).toBeVisible();

	const site = await open('https://personal.example.test/');
	const locked = await popupFor('https://personal.example.test/');
	await expect(locked.getByText('Your personal logins are locked.')).toBeVisible();
	await expect(locked.getByRole('button', { name: `Fill Mine ${run}` })).toHaveCount(0);

	const options = await open(extensionPage('options.html#unlock'));
	const form = options.locator('form').filter({ has: options.getByLabel('Passphrase') });
	await form.getByLabel('Passphrase').fill('not the passphrase');
	await form.getByRole('button', { name: 'Unlock', exact: true }).click();
	await expect(options.getByRole('alert')).toHaveText('That did not unlock your personal logins.');
	await form.getByLabel('Passphrase').fill(passphrase);
	await form.getByRole('button', { name: 'Unlock', exact: true }).click();
	await expect(options.getByRole('status')).toContainText('Unlocked.');

	const popup = await popupFor('https://personal.example.test/');
	await popup.getByRole('button', { name: `Fill Mine ${run}` }).click();
	await expect(site.getByLabel('User')).toHaveValue('bob-himself');
	await expect(site.getByLabel('Password')).toHaveValue(`Mine-Pw-${run}`);
});

test('My account lists the extension and disconnects it', async () => {
	await bob.goto('/account');
	const section = bob.getByRole('region', { name: 'Browser extension' });
	const disconnect = section.getByRole('button', { name: /^Disconnect / });
	await expect(disconnect.first()).toBeVisible();
	// Earlier runs against the development stack leave theirs behind.
	while ((await disconnect.count()) > 0) {
		const before = await disconnect.count();
		await disconnect.first().click();
		await expect(disconnect).toHaveCount(before - 1);
	}
	await expect(section.getByText('No extension is connected.')).toBeVisible();

	await open('https://portal.example.test/');
	const popup = await popupFor('https://portal.example.test/');
	await expect(popup.getByText('The extension is not connected to remotehub yet.')).toBeVisible();
});
