/** Pure helpers to show the flat tree from the API as nested folders. */
import type { Credential, Device, Folder, Tree } from '$lib/api/catalog';

export interface FolderNode {
	folder: Folder;
	folders: FolderNode[];
	devices: Device[];
	credentials: Credential[];
}

/** Nests folders, devices and credentials; siblings are sorted by name. */
export function nest(tree: Tree, locale?: string): FolderNode[] {
	const byName = (a: { name: string }, b: { name: string }) =>
		a.name.localeCompare(b.name, locale, { sensitivity: 'base' });
	const nodes = new Map<string, FolderNode>(
		tree.folders.map((folder) => [folder.id, { folder, folders: [], devices: [], credentials: [] }])
	);
	const roots: FolderNode[] = [];
	for (const node of nodes.values()) {
		const parent = node.folder.parent_id ? nodes.get(node.folder.parent_id) : undefined;
		(parent ? parent.folders : roots).push(node);
	}
	for (const device of tree.devices) nodes.get(device.folder_id)?.devices.push(device);
	for (const credential of tree.credentials)
		nodes.get(credential.folder_id)?.credentials.push(credential);
	for (const node of nodes.values()) {
		node.folders.sort((a, b) => byName(a.folder, b.folder));
		node.devices.sort(byName);
		node.credentials.sort(byName);
	}
	return roots.sort((a, b) => byName(a.folder, b.folder));
}

/** Keeps what matches `query` (name, host, user name) and the folders on the way. */
export function filter(nodes: FolderNode[], query: string): FolderNode[] {
	const q = query.trim().toLowerCase();
	if (!q) return nodes;
	const hit = (...values: string[]) => values.some((v) => v.toLowerCase().includes(q));
	return nodes.flatMap((node) => {
		const folders = filter(node.folders, query);
		const devices = node.devices.filter((d) => hit(d.name, d.host, d.description));
		const credentials = node.credentials.filter((c) => hit(c.name, c.username, c.domain));
		const keep = hit(node.folder.name) || folders.length || devices.length || credentials.length;
		if (!keep) return [];
		// A matching folder shows its whole content.
		return hit(node.folder.name)
			? [node]
			: [{ folder: node.folder, folders, devices, credentials }];
	});
}

/** The folder path from the top down to (and including) a folder. */
export function pathTo(tree: Tree, folderId: string | null): Folder[] {
	const byId = new Map(tree.folders.map((f) => [f.id, f]));
	const path: Folder[] = [];
	let next = folderId ? byId.get(folderId) : undefined;
	while (next && !path.includes(next)) {
		path.unshift(next);
		next = next.parent_id ? byId.get(next.parent_id) : undefined;
	}
	return path;
}
