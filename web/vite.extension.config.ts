/**
 * Builds the browser extension (#201, src/extension) into build-extension/,
 * ready to load unpacked. `pnpm pack:extension` then packs that directory
 * as the ZIP and the signed CRX that remotehub serves.
 *
 * - REMOTEHUB_EXTENSION_VERSION: the release's version; without it the
 *   workspace's version from ../Cargo.toml.
 * - REMOTEHUB_EXTENSION_KEY: the private key (PEM) whose public half goes
 *   into the manifest and fixes the extension's ID. Releases sign with a key
 *   only the maintainer holds; without it the development key in this
 *   directory, which is public on purpose and gives the ID the development
 *   stack allows.
 * - REMOTEHUB_EXTENSION_E2E=1: the build for the end-to-end tests, which may
 *   access every page without being asked (src/extension/manifest.ts).
 */
import { createPrivateKey, createPublicKey } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig, type Plugin } from 'vite';
import { LOCALE_KEYS, manifest } from './src/extension/manifest.ts';

const here = import.meta.dirname;
const source = resolve(here, 'src/extension');

export const DEV_KEY = resolve(here, 'extension-dev-key.pem');

function version(): string {
	const given = process.env.REMOTEHUB_EXTENSION_VERSION;
	if (given) return given;
	const cargo = readFileSync(resolve(here, '../Cargo.toml'), 'utf8');
	const found = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(cargo);
	if (!found) throw new Error('no version in ../Cargo.toml; set REMOTEHUB_EXTENSION_VERSION');
	return found[1];
}

/** The public key of the signing key, as the manifest's `key` takes it. */
function publicKey(): string {
	const pem = readFileSync(process.env.REMOTEHUB_EXTENSION_KEY || DEV_KEY);
	return createPublicKey(createPrivateKey(pem))
		.export({ type: 'spki', format: 'der' })
		.toString('base64');
}

/** manifest.json, the texts of `_locales`, the icons and the policy schema. */
function extensionFiles(): Plugin {
	return {
		name: 'remotehub-extension-files',
		generateBundle() {
			const emit = (fileName: string, source: string | Uint8Array) =>
				this.emitFile({ type: 'asset', fileName, source });
			const testing = process.env.REMOTEHUB_EXTENSION_E2E === '1';
			emit('manifest.json', JSON.stringify(manifest(version(), publicKey(), testing), null, '\t'));
			for (const locale of ['en', 'de']) {
				const messages = JSON.parse(
					readFileSync(resolve(here, `messages/${locale}.json`), 'utf8')
				) as Record<string, string>;
				const chosen = Object.fromEntries(
					LOCALE_KEYS.map((key) => [key, { message: messages[key] }])
				);
				emit(`_locales/${locale}/messages.json`, JSON.stringify(chosen, null, '\t'));
			}
			for (const size of [16, 32, 48, 128]) {
				emit(`icons/${size}.png`, readFileSync(resolve(source, `icons/${size}.png`)));
			}
			emit('managed-schema.json', readFileSync(resolve(source, 'managed-schema.json')));
		}
	};
}

export default defineConfig({
	root: source,
	publicDir: false,
	resolve: { alias: { $lib: resolve(here, 'src/lib') } },
	plugins: [tailwindcss(), svelte({ compilerOptions: { runes: true } }), extensionFiles()],
	build: {
		outDir: resolve(
			here,
			process.env.REMOTEHUB_EXTENSION_E2E === '1' ? 'build-extension-e2e' : 'build-extension'
		),
		emptyOutDir: true,
		target: 'chrome127',
		// No inline scripts or eval: Manifest V3 allows neither.
		modulePreload: false,
		assetsInlineLimit: 0,
		rolldownOptions: {
			input: {
				popup: resolve(source, 'popup.html'),
				options: resolve(source, 'options.html'),
				clipboard: resolve(source, 'clipboard.html'),
				background: resolve(source, 'background.ts')
			},
			output: {
				// The manifest names the service worker by this fixed name.
				entryFileNames: (chunk) =>
					chunk.name === 'background' ? 'background.js' : 'assets/[name]-[hash].js'
			}
		}
	}
});
