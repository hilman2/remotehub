/** Pure helpers to show the flat tree from the API as nested folders and collections. */
import { allows, type Collection, type Device, type Folder, type Tree } from '$lib/api/catalog';

export interface FolderNode {
	folder: Folder;
	folders: FolderNode[];
	devices: Device[];
}

/** Nests folders and devices; siblings are sorted by name. */
export function nest(tree: Tree, locale?: string): FolderNode[] {
	const byName = (a: { name: string }, b: { name: string }) =>
		a.name.localeCompare(b.name, locale, { sensitivity: 'base' });
	const nodes = new Map<string, FolderNode>(
		tree.folders.map((folder) => [folder.id, { folder, folders: [], devices: [] }])
	);
	const roots: FolderNode[] = [];
	for (const node of nodes.values()) {
		const parent = node.folder.parent_id ? nodes.get(node.folder.parent_id) : undefined;
		(parent ? parent.folders : roots).push(node);
	}
	for (const device of tree.devices) nodes.get(device.folder_id)?.devices.push(device);
	for (const node of nodes.values()) {
		node.folders.sort((a, b) => byName(a.folder, b.folder));
		node.devices.sort(byName);
	}
	return roots.sort((a, b) => byName(a.folder, b.folder));
}

/** The folder path from the top down to (and including) a folder. */
export function pathTo(tree: Tree, folderId: string | null): Folder[] {
	return walk(tree.folders, folderId);
}

/** The collection path from the top down to (and including) a collection (#190). */
export function collectionPath(tree: Tree, collectionId: string | null): Collection[] {
	return walk(tree.collections, collectionId);
}

/**
 * The collections a person may put credentials into (edit), each with its
 * path written out ("Shared / Linux"), sorted by that path.
 */
export function collectionPlaces(tree: Tree, locale?: string): { id: string; path: string }[] {
	return tree.collections
		.filter((collection) => allows(collection.role, 'edit'))
		.map((collection) => ({
			id: collection.id,
			path: collectionPath(tree, collection.id)
				.map((c) => c.name)
				.join(' / ')
		}))
		.sort((a, b) => a.path.localeCompare(b.path, locale, { sensitivity: 'base' }));
}

/** A collection and every collection below it, by id. */
export function collectionsBelow(tree: Tree, collectionId: string): Set<string> {
	const inside = new Set([collectionId]);
	let grew = true;
	while (grew) {
		grew = false;
		for (const collection of tree.collections) {
			if (collection.parent_id && inside.has(collection.parent_id) && !inside.has(collection.id)) {
				inside.add(collection.id);
				grew = true;
			}
		}
	}
	return inside;
}

function walk<T extends { id: string; parent_id: string | null }>(
	items: T[],
	id: string | null
): T[] {
	const byId = new Map(items.map((item) => [item.id, item]));
	const path: T[] = [];
	let next = id ? byId.get(id) : undefined;
	while (next && !path.includes(next)) {
		path.unshift(next);
		next = next.parent_id ? byId.get(next.parent_id) : undefined;
	}
	return path;
}

/**
 * The site connector a folder passes on to what is in it (#176): its own,
 * or the nearest one above it; null: none, devices connect directly. As the
 * server's `folder_connector`.
 */
export function folderConnector(tree: Tree, folderId: string | null): string | null {
	const path = pathTo(tree, folderId);
	for (let i = path.length - 1; i >= 0; i--) {
		if (path[i].connector_id) return path[i].connector_id;
	}
	return null;
}
