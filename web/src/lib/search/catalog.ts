/** The device catalog as search items: devices, credentials and folders. */
import type { ObjectKind, Protocol, Tree } from '$lib/api/catalog';
import { pathTo } from '$lib/catalog/tree';
import type { Searchable } from './rank';

/** One result as the list shows it. */
export interface Hit {
	kind: ObjectKind;
	id: string;
	name: string;
	/** The folder path above it. */
	where: string;
	/** Host or user name. */
	detail: string;
	protocol: Protocol | null;
}

export const pickKey = (kind: ObjectKind, id: string) => `${kind}:${id}`;

export function catalogItems(tree: Tree): Searchable<Hit>[] {
	const where = (folderId: string | null) =>
		pathTo(tree, folderId)
			.map((f) => f.name)
			.join(' / ');
	const item = (
		kind: ObjectKind,
		id: string,
		name: string,
		folder: string | null,
		detail: string,
		more: { text: string; weight: number }[],
		protocol: Protocol | null = null
	): Searchable<Hit> => {
		const path = where(folder);
		return {
			key: pickKey(kind, id),
			name,
			fields: [
				{ text: name, weight: 1 },
				...more,
				// A folder's name finds what is in it, but only weakly.
				{ text: path, weight: 0.4 }
			],
			item: { kind, id, name, where: path, detail, protocol }
		};
	};
	return [
		...tree.devices.map((d) =>
			item(
				'device',
				d.id,
				d.name,
				d.folder_id,
				d.host,
				[
					{ text: d.host, weight: 0.8 },
					{ text: d.description, weight: 0.5 }
				],
				d.protocol
			)
		),
		...tree.credentials.map((c) =>
			item('credential', c.id, c.name, c.folder_id, c.username, [
				{ text: c.username, weight: 0.8 },
				{ text: c.domain, weight: 0.6 }
			])
		),
		...tree.folders.map((f) => item('folder', f.id, f.name, f.parent_id, '', []))
	];
}
