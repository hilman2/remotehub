/**
 * Guards against untranslated text: every piece of text a person reads or
 * hears must come from a message (messages/{locale}.json), never be written
 * into a component.
 *
 * - Every .svelte file is parsed with the Svelte compiler; visible text and
 *   user-facing attributes (title, alt, aria-label …) must not contain words.
 * - Every message key must be used somewhere, so catalogs don't rot.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parse } from 'svelte/compiler';
import { describe, expect, it } from 'vitest';
import en from '../../messages/en.json';

const SRC = join(import.meta.dirname, '..');
const GENERATED = /^lib[/\\]paraglide[/\\]/;

/** Attributes (and component props) whose value people read or hear. */
const USER_FACING_ATTRIBUTES = new Set([
	'title',
	'alt',
	'placeholder',
	'label',
	'aria-label',
	'aria-description',
	'aria-placeholder',
	'aria-roledescription',
	'aria-valuetext',
	'caption',
	'description',
	'heading',
	'message',
	'range',
	'text'
]);

/** Words that are never translated: the product name. */
const UNTRANSLATED_WORDS = /remotehub/g;

export interface RawText {
	line: number;
	text: string;
}

function needsTranslation(text: string): boolean {
	return /\p{L}/u.test(text.replace(UNTRANSLATED_WORDS, ''));
}

/** Finds hard-coded, user-facing text in a Svelte component. */
function findRawText(source: string): RawText[] {
	const found: RawText[] = [];
	const lineOf = (offset: number) => source.slice(0, offset).split('\n').length;

	const walk = (node: unknown): void => {
		if (Array.isArray(node)) {
			node.forEach(walk);
			return;
		}
		if (!node || typeof node !== 'object') return;
		const n = node as {
			type?: string;
			name?: string;
			value?: unknown;
			start?: number;
			data?: string;
		};

		switch (n.type) {
			case 'Text':
				if (n.data && needsTranslation(n.data)) {
					found.push({ line: lineOf(n.start ?? 0), text: n.data.trim() });
				}
				return;
			case 'Attribute':
				// Only literal parts of user-facing attributes; {expressions} are code.
				if (USER_FACING_ATTRIBUTES.has(n.name ?? '') && Array.isArray(n.value)) walk(n.value);
				return;
			case 'Comment':
			case 'ExpressionTag':
			case 'SpreadAttribute':
				return;
		}
		// Directives (style:, class:, bind:, on: …) carry code or CSS, not text.
		if (n.type?.endsWith('Directive')) return;

		for (const [key, value] of Object.entries(n)) {
			if (key !== 'metadata' && key !== 'parent') walk(value);
		}
	};

	// Only the markup: <script> and <style> are code.
	walk(parse(source, { modern: true }).fragment);
	return found;
}

function sourceFiles(extensions: RegExp): string[] {
	return (readdirSync(SRC, { recursive: true }) as string[])
		.filter((file) => extensions.test(file) && !GENERATED.test(file))
		.sort();
}

describe('the guard itself', () => {
	it('finds visible text and user-facing attributes, but not code or the product name', () => {
		const found = findRawText(
			`<script>const hint = 'not markup';</script>
			<h1>remotehub</h1>
			<p>Hello</p>
			<p>{m.greeting()} · 42</p>
			<img alt="Logo" class="size-6" style:color="var(--s-{state})" />
			<button title={m.save()} aria-label="Close">×</button>`
		);
		expect(found.map((f) => f.text)).toEqual(['Hello', 'Logo', 'Close']);
	});
});

describe('no untranslated text in components', () => {
	it.each(sourceFiles(/\.svelte$/))('%s', (file) => {
		const found = findRawText(readFileSync(join(SRC, file), 'utf8'));
		expect(found, 'use a message from messages/{locale}.json instead').toEqual([]);
	});
});

describe('message catalogs', () => {
	it('have no unused keys', () => {
		const used = new Set<string>();
		for (const file of sourceFiles(/\.(svelte|ts)$/)) {
			for (const match of readFileSync(join(SRC, file), 'utf8').matchAll(/\bm\.(\w+)/g)) {
				used.add(match[1]);
			}
		}
		const unused = Object.keys(en).filter((key) => key !== '$schema' && !used.has(key));
		expect(unused).toEqual([]);
	});
});
