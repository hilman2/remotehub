import { describe, expect, it } from 'vitest';
import { allows, type Device, type Tree } from '$lib/api/catalog';
import { catalogItems } from '$lib/search/catalog';
import { rank } from '$lib/search/rank';
import { nest, pathTo } from './tree';

function device(id: string, name: string, host: string): Device {
	return {
		id,
		folder_id: 'l',
		name,
		protocol: 'ssh',
		host,
		port: 22,
		auth_mode: 'ask',
		credential_id: null,
		description: '',
		keyboard_layout: null,
		certificate_fingerprint: null,
		host_key_fingerprint: null,
		connector_id: null,
		role: 'edit'
	};
}

const tree: Tree = {
	may_create_top_level: true,
	open: [],
	purpose_required: false,
	folders: [
		{ id: 'l', parent_id: 's', name: 'Linux', role: 'edit' },
		{ id: 's', parent_id: null, name: 'Servers', role: 'connect' },
		{ id: 'n', parent_id: null, name: 'Network', role: null },
		{ id: 'w', parent_id: 's', name: 'windows', role: 'connect' }
	],
	devices: [device('d2', 'web02', 'web02.example.com'), device('d1', 'Web01', 'db.example.com')],
	credentials: [
		{
			id: 'c',
			folder_id: 'w',
			name: 'domain admin',
			kind: 'password',
			username: 'administrator',
			domain: 'EXAMPLE',
			version: 1,
			key_algorithm: null,
			key_fingerprint: null,
			has_certificate: false,
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
		expect(servers.folders[1].credentials.map((c) => c.name)).toEqual(['domain admin']);
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

	it('finds credentials by user name and what is in a folder by its name', () => {
		expect(find('ADMINISTRATOR')).toEqual(['credential:domain admin']);
		// The folder itself first, then its content.
		expect(find('linux')).toEqual(['folder:Linux', 'device:Web01', 'device:web02']);
		expect(find('nothing matches')).toEqual([]);
	});
});

describe('pathTo', () => {
	it('lists the folders from the top', () => {
		expect(pathTo(tree, 'l').map((f) => f.name)).toEqual(['Servers', 'Linux']);
		expect(pathTo(tree, null)).toEqual([]);
	});
});

describe('allows', () => {
	it('orders the roles', () => {
		expect(allows('edit', 'connect')).toBe(true);
		expect(allows('connect', 'edit')).toBe(false);
		expect(allows(null, 'list')).toBe(false);
	});
});
