/**
 * The vault's list (#190, #193): personal entries and shared credentials as
 * one table, narrowed to the folder or view chosen and sorted by a column,
 * as in KeePass.
 */
import type { Collection, Credential, Tree } from '$lib/api/catalog';
import { collectionPath } from '$lib/catalog/tree';
import { pickKey } from '$lib/search/catalog';
import { frequent, rank, type Pick } from '$lib/search/rank';
import { totpOf } from './totp';
import type { Entry } from './vault';

/** Where the owner looks. */
export type Scope =
	/** Everything but the recycle bins. */
	| { kind: 'all' }
	| { kind: 'recent' }
	/** Run out, or running out within `SOON` days. */
	| { kind: 'expiring' }
	/** A personal folder and what is below it; null: the whole personal vault. */
	| { kind: 'personal'; folder: string | null }
	| { kind: 'collection'; id: string }
	| { kind: 'bin'; side: 'personal' | 'shared' };

/** The columns the table sorts by. */
export type Column = 'title' | 'username' | 'url' | 'notes' | 'changed';
export interface Sort {
	column: Column;
	descending: boolean;
}

interface Base {
	/** What picks remember it by: the entry's id, or the credential's pick key. */
	key: string;
	id: string;
	title: string;
	username: string;
	url: string;
	notes: string;
	icon: number;
	/** The path of the folder or collection it lies in; empty at the personal top. */
	where: string;
	tags: string[];
	/** `YYYY-MM-DD`; null: never runs out. */
	expires: string | null;
	/** RFC 3339: when it was last changed; null: not known. */
	changed: string | null;
	/** In a recycle bin. */
	deleted: boolean;
	hasTotp: boolean;
	files: number;
}

export type Item =
	| (Base & { source: 'personal'; folder: string | null; entry: Entry })
	| (Base & { source: 'shared'; collection: string; credential: Credential });

/** A node of the sidebar's outline, with how deep it sits. */
export interface OutlineNode {
	id: string;
	name: string;
	depth: number;
}

/** How many days ahead "expiring" looks. */
export const SOON = 14;

/** Today as `YYYY-MM-DD` in the local calendar, as expiry dates are written. */
export function today(now = new Date()): string {
	return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
}

/** The day `days` after `day` (`YYYY-MM-DD`). */
function later(day: string, days: number): string {
	const [year, month, date] = day.split('-').map(Number);
	return today(new Date(year, month - 1, date + days));
}

/** Whether the item's password has run out by `day`. */
export const isExpired = (item: Base, day: string) => !!item.expires && item.expires <= day;

/** Whether it has run out or will within `SOON` days of `day`. */
export const isExpiring = (item: Base, day: string) =>
	!!item.expires && item.expires <= later(day, SOON);

const byName = (locale?: string) => (a: { name: string }, b: { name: string }) =>
	a.name.localeCompare(b.name, locale, { sensitivity: 'base' });

/** Nodes of a tree in reading order: each followed by what is below it. */
export function outline(
	nodes: { id: string; parent_id: string | null; name: string }[],
	locale?: string
): OutlineNode[] {
	const ids = new Set(nodes.map((node) => node.id));
	const out: OutlineNode[] = [];
	const visit = (parent: string | null, depth: number) => {
		const below = nodes
			.filter((node) =>
				parent === null
					? node.parent_id === null || !ids.has(node.parent_id)
					: node.parent_id === parent
			)
			.sort(byName(locale));
		for (const node of below) {
			// A loop in the data would recurse forever.
			if (out.some((seen) => seen.id === node.id)) continue;
			out.push({ id: node.id, name: node.name, depth });
			visit(node.id, depth + 1);
		}
	};
	visit(null, 0);
	return out;
}

/** The personal vault's folders as tree nodes. */
export function personalFolders(
	entries: Entry[]
): { id: string; parent_id: string | null; name: string }[] {
	return entries
		.filter((entry) => entry.content?.kind === 'folder')
		.map((entry) => ({
			id: entry.id,
			parent_id: entry.content?.parent ?? null,
			name: entry.content?.title ?? ''
		}));
}

/** A folder and every folder below it, by id; works for either tree. */
function below(nodes: { id: string; parent_id: string | null }[], id: string): Set<string> {
	const inside = new Set([id]);
	let grew = true;
	while (grew) {
		grew = false;
		for (const node of nodes) {
			if (node.parent_id && inside.has(node.parent_id) && !inside.has(node.id)) {
				inside.add(node.id);
				grew = true;
			}
		}
	}
	return inside;
}

/** A personal entry's one-time password: its own, or one kept in a field before #193. */
export function personalTotp(entry: Entry): string {
	const content = entry.content;
	if (!content) return '';
	if (content.totp) return content.totp;
	const field = (content.fields ?? []).find((f) => totpOf(f.name, f.value));
	return field?.value ?? '';
}

/** The readable personal entries as list items; folders are not items. */
export function personalItems(entries: Entry[]): Item[] {
	const folders = personalFolders(entries);
	const path = (id: string | null) => {
		const names: string[] = [];
		let at = folders.find((f) => f.id === id);
		while (at && names.length < 20) {
			names.unshift(at.name);
			at = folders.find((f) => f.id === at?.parent_id);
		}
		return names.join(' / ');
	};
	return entries.flatMap((entry): Item[] => {
		const content = entry.content;
		if (!content || content.kind === 'folder') return [];
		const folder = content.parent ?? null;
		return [
			{
				source: 'personal',
				key: entry.id,
				id: entry.id,
				title: content.title,
				username: content.username,
				url: content.url,
				notes: content.notes,
				icon: content.icon ?? 0,
				where: path(folder),
				tags: content.tags ?? [],
				expires: content.expires ?? null,
				changed: content.changed ?? content.history?.[0]?.at ?? null,
				deleted: !!content.deleted,
				hasTotp: !!personalTotp(entry),
				files: (content.attachments ?? []).length,
				folder,
				entry
			}
		];
	});
}

/** The shared credentials this user sees, as list items. */
export function sharedItems(tree: Tree): Item[] {
	return tree.credentials.map((credential): Item => ({
		source: 'shared',
		key: pickKey('credential', credential.id),
		id: credential.id,
		title: credential.name,
		username: credential.username,
		url: credential.url,
		notes: credential.notes,
		icon: credential.icon,
		where: collectionPath(tree, credential.collection_id)
			.map((c) => c.name)
			.join(' / '),
		tags: credential.tags,
		expires: credential.expires_on,
		changed: credential.updated_at,
		deleted: !!credential.deleted_at,
		hasTotp: credential.has_totp,
		files: credential.attachments.length,
		collection: credential.collection_id,
		credential
	}));
}

/** How many recently used entries "Recent" shows. */
const RECENT = 20;

interface Context {
	folders: { id: string; parent_id: string | null }[];
	collections: Collection[];
	recent: string[];
	/** `YYYY-MM-DD`. */
	today: string;
}

/** The folders or collections `scope` reaches down to; null for a scope that is none. */
function reach(scope: Scope, context: Context): Set<string> | null {
	if (scope.kind === 'personal' && scope.folder) return below(context.folders, scope.folder);
	if (scope.kind === 'collection') return below(context.collections, scope.id);
	return null;
}

/**
 * Whether `item` belongs in `scope`. Folders and collections reach down
 * into what lies below (`inside`, computed once per scope); only the bins
 * show what was deleted.
 */
export function inScope(
	item: Item,
	scope: Scope,
	context: Context,
	inside: Set<string> | null = reach(scope, context)
): boolean {
	if (scope.kind === 'bin') return item.deleted && item.source === scope.side;
	if (item.deleted) return false;
	switch (scope.kind) {
		case 'all':
			return true;
		case 'recent':
			return context.recent.includes(item.key);
		case 'expiring':
			return isExpiring(item, context.today);
		case 'personal':
			return (
				item.source === 'personal' &&
				(scope.folder === null || (item.folder !== null && !!inside?.has(item.folder)))
			);
		case 'collection':
			return item.source === 'shared' && !!inside?.has(item.collection);
	}
}

/** The keys used most lately, for "Recent" and sorting by use. */
export const recentKeys = (picks: Pick[], now: number) => frequent(picks, now, RECENT);

export interface ListOptions {
	scope: Scope;
	sort: Sort;
	query: string;
	picks: Pick[];
	now: number;
	folders: { id: string; parent_id: string | null }[];
	collections: Collection[];
	/** `YYYY-MM-DD`; today by default. */
	today?: string;
	locale?: string;
}

/**
 * The items to list: with a query, what matches it best first, across the
 * whole vault but its bins; otherwise those in `scope`, sorted by the
 * column chosen. "Recent" keeps the order of use.
 */
export function listed(items: Item[], options: ListOptions): Item[] {
	const { picks, now, locale } = options;
	if (options.query.trim()) {
		return rank(
			items
				.filter((item) => !item.deleted)
				.map((item) => ({
					key: item.key,
					name: item.title,
					fields: [
						{ text: item.title, weight: 1 },
						{ text: item.username, weight: 0.8 },
						{ text: item.url, weight: 0.7 },
						{ text: item.tags.join(' '), weight: 0.7 },
						{ text: item.notes, weight: 0.5 },
						{ text: item.where, weight: 0.4 }
					],
					item
				})),
			options.query,
			picks,
			now,
			locale
		);
	}
	const order = frequent(picks, now, items.length);
	const context: Context = {
		folders: options.folders,
		collections: options.collections,
		recent: order.slice(0, RECENT),
		today: options.today ?? today()
	};
	const inside = reach(options.scope, context);
	const shown = items.filter((item) => inScope(item, options.scope, context, inside));
	if (options.scope.kind === 'recent') {
		return shown.sort((a, b) => order.indexOf(a.key) - order.indexOf(b.key));
	}
	const { column, descending } = options.sort;
	const text = (item: Item) => (column === 'changed' ? (item.changed ?? '') : item[column]);
	const title = (a: Item, b: Item) =>
		a.title.localeCompare(b.title, locale, { sensitivity: 'base' });
	return shown.sort((a, b) => {
		const compared =
			column === 'changed'
				? text(a).localeCompare(text(b))
				: text(a).localeCompare(text(b), locale, { sensitivity: 'base' });
		return (descending ? -compared : compared) || title(a, b);
	});
}

/** How many items each scope holds, for the sidebar. */
export function counter(
	items: Item[],
	context: {
		folders: { id: string; parent_id: string | null }[];
		collections: Collection[];
		picks: Pick[];
		now: number;
		today?: string;
	}
): (scope: Scope) => number {
	const inside: Context = {
		folders: context.folders,
		collections: context.collections,
		recent: recentKeys(context.picks, context.now),
		today: context.today ?? today()
	};
	// Each folder and collection counts what lies in it and below, added up
	// once along every entry's path: a sidebar of a hundred folders asks for
	// a hundred counts at every change.
	const parent = new Map<string, string | null>([
		...context.folders.map((f): [string, string | null] => [
			`p:${f.id}`,
			f.parent_id && `p:${f.parent_id}`
		]),
		...context.collections.map((c): [string, string | null] => [
			`s:${c.id}`,
			c.parent_id && `s:${c.parent_id}`
		])
	]);
	const within = new Map<string, number>();
	for (const item of items) {
		if (item.deleted) continue;
		let at: string | null =
			item.source === 'personal' ? item.folder && `p:${item.folder}` : `s:${item.collection}`;
		const seen = new Set<string>();
		while (at && !seen.has(at)) {
			seen.add(at);
			within.set(at, (within.get(at) ?? 0) + 1);
			at = parent.get(at) ?? null;
		}
	}
	return (scope) => {
		if (scope.kind === 'personal' && scope.folder) return within.get(`p:${scope.folder}`) ?? 0;
		if (scope.kind === 'collection') return within.get(`s:${scope.id}`) ?? 0;
		return items.filter((item) => inScope(item, scope, inside, null)).length;
	};
}
