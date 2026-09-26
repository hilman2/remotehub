import { paraglideVitePlugin } from '@inlang/paraglide-js';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';

// On Windows, file events from bind mounts do not reach containers;
// compose.dev.yml therefore sets VITE_POLLING=1.
const polling = process.env.VITE_POLLING === '1';

// The server on the compose network; in development, API and WebSocket go through Vite.
const server = process.env.REMOTEHUB_SERVER ?? 'http://server:8080';

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Runes everywhere except in libraries. Can go with Svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			// SPA: every route falls back to index.html, served by the server.
			adapter: adapter({ fallback: 'index.html' }),

			// One app version for everything a build runs. SvelteKit's default is a
			// timestamp per process; the CI builds while vitest syncs in parallel, and
			// two versions give a start page that crashes (#49). The CI sets the
			// commit, a release its version; development keeps the default.
			...(process.env.REMOTEHUB_WEB_VERSION
				? { version: { name: process.env.REMOTEHUB_WEB_VERSION } }
				: {})
		}),

		// Messages in messages/{locale}.json, compiled to typed functions. The
		// locale comes from the user's choice (localStorage), then the browser,
		// then English — never from the URL, the SPA has one set of routes.
		// Keep the strategy in sync with the `i18n` script in package.json.
		// Not under vitest, and not when the CI compiled them beforehand
		// (PARAGLIDE_PRECOMPILED): then vitest and the build leave the compiled
		// messages alone while svelte-check reads them in parallel.
		...(process.env.VITEST || process.env.PARAGLIDE_PRECOMPILED
			? []
			: [
					paraglideVitePlugin({
						project: './project.inlang',
						outdir: './src/lib/paraglide',
						strategy: ['localStorage', 'preferredLanguage', 'baseLocale'],
						emitTsDeclarations: true
					})
				])
	],
	server: {
		host: '0.0.0.0',
		// 5173 is taken by another project on the development machine.
		port: 5180,
		strictPort: true,
		watch: polling ? { usePolling: true, interval: 500 } : undefined,
		proxy: {
			'/api': { target: server, changeOrigin: true, ws: true },
			// The root certificate of Caddy's own CA (#146).
			'^/ca\\.(crt|cer)$': { target: server, changeOrigin: true },
			// The site connector for Windows (#188).
			'/downloads': { target: server, changeOrigin: true }
		}
	},
	test: {
		expect: { requireAssertions: true },
		projects: [
			{
				extends: './vite.config.ts',
				test: {
					name: 'server',
					environment: 'node',
					include: ['src/**/*.{test,spec}.{js,ts}'],
					exclude: ['src/**/*.svelte.{test,spec}.{js,ts}']
				}
			}
		]
	}
});
