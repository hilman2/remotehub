import { defineConfig, devices } from '@playwright/test';

// End-to-end tests in a real browser against a running remotehub with the
// test lab (deploy/testlab). Development:
//   docker compose -f deploy/compose.dev.yml --profile e2e run --rm e2e
// The CI runs them in full runs (scripts/ci/lokal.sh). Keep @playwright/test
// equal to the Playwright image in deploy/compose.dev.yml and lokal.sh.
export default defineConfig({
	testDir: 'tests/e2e',
	timeout: 90_000,
	expect: { timeout: 15_000 },
	retries: 0,
	reporter: [['list']],
	use: {
		...devices['Desktop Chrome'],
		// Secure cookies need localhost (or HTTPS): the runner shares the
		// server's network, so this is always localhost.
		baseURL: process.env.E2E_BASE_URL ?? 'http://localhost:5180',
		locale: 'en-GB',
		trace: 'retain-on-failure'
	},
	// A fresh database goes through the setup wizard first (#143): it makes
	// the administrators the other tests sign in as.
	projects: [
		{ name: 'setup', testMatch: /setup\.spec\.ts/ },
		{ name: 'e2e', testIgnore: /setup\.spec\.ts/, dependencies: ['setup'] }
	]
});
