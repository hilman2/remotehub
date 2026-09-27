/**
 * The vault's list (#190): personal entries and shared credentials as one
 * list, narrowed to where the owner looks and sorted as they choose.
 */
import type { Collection, Credential, Tree } from '$lib/api/catalog';
import { collectionPath } from '$lib/catalog/tree';
import { pickKey } from '$lib/search/catalog';
import { frequent, rank, type Pick } from '$lib/search/rank';
import { totpOf } from './totp';
import type { Entry } from './vault';

/** Kinds of entries the sidebar filters by. */
export type Kind = 'logins' | 'keys' | 'totp' | 'files';

/** Where the owner looks. */
export type Scope =
	| { kind: 'all' }
	| { kind: 'recent' }
	/** A personal folder and what is below it; null: the whole personal vault. */
	| { kind: 'personal'; folder: string | null }
	| { kind: 'collection'; id: string }
	| { kind: 'filter'; filter: Kind };

export type Sort = 'name' | 'used' | 'place';

interface Base {
	/** What picks remember it by: the entry's id, or the credential's pick key. */
	key: string;
	id: string;
	title: string;
	username: string;
	url: string;
	icon: number;
	/** The path of the folder or collection it lies in; empty at the personal top. */
	where: string;
	kinds: Kind[];
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

const isTotp = (field: { name: string; value?: string }) =>
	/otp|totp/i.test(field.name) || !!totpOf(field.name, field.value ?? '');

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
		const kinds: Kind[] = ['logins'];
		if ((content.fields ?? []).some(isTotp)) kinds.push('totp');
		if ((content.attachments ?? []).length > 0) kinds.push('files');
		const folder = content.parent ?? null;
		return [
			{
				source: 'personal',
				key: entry.id,
				id: entry.id,
				title: content.title,
				username: content.username,
				url: content.url,
				icon: content.icon ?? 0,
				where: path(folder),
				kinds,
				folder,
				entry
			}
		];
	});
}

/** The shared credentials this user sees, as list items. */
export function sharedItems(tree: Tree): Item[] {
	return tree.credentials.map((credential): Item => {
		const kinds: Kind[] = [credential.kind === 'ssh_key' ? 'keys' : 'logins'];
		if (credential.fields.some(isTotp)) kinds.push('totp');
		if (credential.attachments.length > 0) kinds.push('files');
		return {
			source: 'shared',
			key: pickKey('credential', credential.id),
			id: credential.id,
			title: credential.name,
			username: credential.domain
				? `${credential.domain}\\${credential.username}`
				: credential.username,
			url: credential.url,
			icon: credential.icon,
			where: collectionPath(tree, credential.collection_id)
				.map((c) => c.name)
				.join(' / '),
			kinds,
			collection: credential.collection_id,
			credential
		};
	});
}

/** How many recently used entries "Recent" shows. */
const RECENT = 20;

/**
 * Whether `item` belongs in `scope`. `folders` are the personal folders,
 * `collections` the tree's; both reach down into what lies below.
 */
export function inScope(
	item: Item,
	scope: Scope,
	context: {
		folders: { id: string; parent_id: string | null }[];
		collections: Collection[];
		recent: string[];
	}
): boolean {
	switch (scope.kind) {
		case 'all':
			return true;
		case 'recent':
			return context.recent.includes(item.key);
		case 'filter':
			return item.kinds.includes(scope.filter);
		case 'personal':
			return (
				item.source === 'personal' &&
				(scope.folder === null ||
					(item.folder !== null && below(context.folders, scope.folder).has(item.folder)))
			);
		case 'collection':
			return item.source === 'shared' && below(context.collections, scope.id).has(item.collection);
	}
}

/** The keys used most lately, for "Recent" and sorting by use. */
export const recentKeys = (picks: Pick[], now: number) => frequent(picks, now, RECENT);

/**
 * The items to list: with a query, what matches it best first, across the
 * whole vault; otherwise those in `scope`, in the order chosen.
 */
export function listed(
	items: Item[],
	options: {
		scope: Scope;
		sort: Sort;
		query: string;
		picks: Pick[];
		now: number;
		folders: { id: string; parent_id: string | null }[];
		collections: Collection[];
		locale?: string;
	}
): Item[] {
	const { picks, now, locale } = options;
	if (options.query.trim()) {
		return rank(
			items.map((item) => ({
				key: item.key,
				name: item.title,
				fields: [
					{ text: item.title, weight: 1 },
					{ text: item.username, weight: 0.8 },
					{ text: item.url, weight: 0.7 },
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
	const recent = order.slice(0, RECENT);
	const context = { folders: options.folders, collections: options.collections, recent };
	const shown = items.filter((item) => inScope(item, options.scope, context));
	const name = (a: Item, b: Item) =>
		a.title.localeCompare(b.title, locale, { sensitivity: 'base' });
	const used = (item: Item) => {
		const at = order.indexOf(item.key);
		return at < 0 ? order.length : at;
	};
	const sort = options.scope.kind === 'recent' ? 'used' : options.sort;
	return shown.sort((a, b) => {
		if (sort === 'used') return used(a) - used(b) || name(a, b);
		if (sort === 'place') {
			const place = (item: Item) => `${item.source === 'personal' ? 0 : 1}${item.where}`;
			return place(a).localeCompare(place(b), locale, { sensitivity: 'base' }) || name(a, b);
		}
		return name(a, b);
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
	}
): (scope: Scope) => number {
	const inside = { ...context, recent: recentKeys(context.picks, context.now) };
	return (scope) => items.filter((item) => inScope(item, scope, inside)).length;
}
