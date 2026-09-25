import { expect, test } from '@playwright/test';

// The certificate on the settings page (#146): Caddy of the ops package with
// what it serves, or a reverse proxy of your own that holds it. The CI's
// server runs without Caddy, the development stack with the lab's.

test('the settings page says what serves the certificate', async ({ page }) => {
	await page.goto('/');
	await expect(page).toHaveURL(/\/sign-in$/);
	await page.getByLabel('User name').fill('alice');
	await page.getByLabel('Password').fill('Alice-Passw0rd!');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();

	await page.goto('/settings#certificate');
	const section = page.getByRole('region', { name: 'Certificate' });
	await expect(section).toBeVisible();
	await expect(
		section.getByTestId('certificate-external').or(section.getByTestId('certificate-served'))
	).toBeVisible();
});
