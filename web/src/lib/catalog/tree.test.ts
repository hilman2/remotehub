import { describe, expect, it } from 'vitest';
import { allows, type Device, type Profile, type Tree } from '$lib/api/catalog';
import { catalogItems } from '$lib/search/catalog';
import { rank } from '$lib/search/rank';
import {
	collectionPath,
	collectionsBelow,
	folderConnector,
	nest,
	pathTo,
	profilesWithin
} from './tree';

function device(id: string, name: string, host: string): Device {
	return {
		id,
		folder_id: 'l',
		name,
		protocol: 'ssh',
		host,
		port: 22,
		auth_mode: 'ask',
		profile_id: null,
		description: '',
		keyboard_layout: null,
		certificate_fingerprint: null,
		host_key_fingerprint: null,
		connector_mode: 'inherit',
		connector_id: null,
		reached_through: null,
		username: '',
		domain: '',
		secret_kind: 'password',
		key_algorithm: null,
		key_fingerprint: null,
		has_certificate: false,
		role: 'edit'
	};
}

function profile(id: string, folder: string | null, name: string): Profile {
	return {
		id,
		folder_id: folder,
		name,
		username: 'admin',
		domain: '',
		secret_kind: 'password',
		key_algorithm: null,
		key_fingerprint: null,
		has_certificate: false,
		updated_at: '2026-09-27T00:00:00Z',
		role: 'connect'
	};
}

const tree: Tree = {
	may_create_top_level: true,
	open: [],
	purpose_required: false,
	folders: [
		{ id: 'l', parent_id: 's', name: 'Linux', role: 'edit', connector_id: null },
		{ id: 's', parent_id: null, name: 'Servers', role: 'connect', connector_id: 'site' },
		{ id: 'n', parent_id: null, name: 'Network', role: null, connector_id: null },
		{ id: 'w', parent_id: 's', name: 'windows', role: 'connect', connector_id: 'office' }
	],
	devices: [device('d2', 'web02', 'web02.example.com'), device('d1', 'Web01', 'db.example.com')],
	profiles: [
		profile('p-servers', 's', 'Server admin'),
		profile('p-linux', 'l', 'Linux root'),
		profile('p-windows', 'w', 'Windows admin'),
		profile('p-top', null, 'Local admin')
	],
	collections: [
		{ id: 'k', parent_id: null, name: 'Keys', role: 'connect' },
		{ id: 'kw', parent_id: 'k', name: 'Windows', role: 'connect' },
		{ id: 'x', parent_id: null, name: 'Other', role: null }
	],
	credentials: [
		{
			id: 'c',
			collection_id: 'kw',
			name: 'domain admin',
			username: 'EXAMPLE\\administrator',
			version: 1,
			url: '',
			notes: '',
			icon: 0,
			fields: [],
			attachments: [],
			tags: [],
			expires_on: null,
			has_totp: false,
			updated_at: '2026-09-27T00:00:00Z',
			deleted_at: null,
			role: 'connect'
		}
	]
};

describe('nest', () => {
	it('builds the folder tree sorted by name, ignoring case', () => {
		const roots = nest(tree, 'en');
		expect(roots.map((n) => n.folder.name)).toEqual(['Network', 'Servers']);
		const servers = roots[1];
		expect(servers.folders.map((n) => n.folder.name)).toEqual(['Linux', 'windows']);
		expect(servers.folders[0].devices.map((d) => d.name)).toEqual(['Web01', 'web02']);
	});
});

describe('collections', () => {
	it('have paths and subtrees of their own, apart from the folders', () => {
		expect(collectionPath(tree, 'kw').map((c) => c.name)).toEqual(['Keys', 'Windows']);
		expect([...collectionsBelow(tree, 'k')].sort()).toEqual(['k', 'kw']);
		expect([...collectionsBelow(tree, 'x')]).toEqual(['x']);
	});
});

describe('searching the catalog', () => {
	const find = (query: string) =>
		rank(catalogItems(tree), query, [], 0, 'en').map((hit) => `${hit.kind}:${hit.name}`);

	it('finds devices by host, with the folder path', () => {
		expect(find('db.example')).toEqual(['device:Web01']);
		const [hit] = rank(catalogItems(tree), 'db.example', [], 0);
		expect(hit.where).toBe('Servers / Linux');
		expect(hit.detail).toBe('db.example.com');
	});

	it('leaves credentials to the vault and finds what is in a folder by its name', () => {
		expect(find('ADMINISTRATOR')).toEqual([]);
		// The folder itself first, then its content.
		expect(find('linux')).toEqual(['folder:Linux', 'device:Web01', 'device:web02']);
		expect(find('nothing matches')).toEqual([]);
	});
});

describe('profilesWithin', () => {
	it('offers the profiles of the folder, the folders above and the top, not a sibling', () => {
		const names = (folder: string) =>
			profilesWithin(tree, folder)
				.map((p) => p.name)
				.sort();
		expect(names('l')).toEqual(['Linux root', 'Local admin', 'Server admin']);
		expect(names('s')).toEqual(['Local admin', 'Server admin']);
		expect(names('n')).toEqual(['Local admin']);
	});
});

describe('pathTo', () => {
	it('lists the folders from the top', () => {
		expect(pathTo(tree, 'l').map((f) => f.name)).toEqual(['Servers', 'Linux']);
		expect(pathTo(tree, null)).toEqual([]);
	});
});

describe('folderConnector', () => {
	it('takes the nearest connector up the path', () => {
		expect(folderConnector(tree, 'l')).toBe('site');
		expect(folderConnector(tree, 'w')).toBe('office');
		expect(folderConnector(tree, 'n')).toBeNull();
		expect(folderConnector(tree, null)).toBeNull();
	});
});

describe('allows', () => {
	it('orders the roles', () => {
		expect(allows('edit', 'connect')).toBe(true);
		expect(allows('connect', 'edit')).toBe(false);
		expect(allows(null, 'list')).toBe(false);
	});
});
