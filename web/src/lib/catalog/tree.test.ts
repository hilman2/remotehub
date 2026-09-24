import { describe, expect, it } from 'vitest';
import { allows, type Device, type Tree } from '$lib/api/catalog';
import { filter, nest, pathTo } from './tree';

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
		host_key_fingerprint: null,
		role: 'edit'
	};
}

const tree: Tree = {
	may_create_top_level: true,
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

describe('filter', () => {
	it('keeps matches and the folders on the way', () => {
		const roots = filter(nest(tree), 'db.example');
		expect(roots.map((n) => n.folder.name)).toEqual(['Servers']);
		expect(roots[0].folders.map((n) => n.folder.name)).toEqual(['Linux']);
		expect(roots[0].folders[0].devices.map((d) => d.name)).toEqual(['Web01']);
	});

	it('finds credentials by user name and shows a matching folder whole', () => {
		expect(filter(nest(tree), 'ADMINISTRATOR')[0].folders[0].credentials).toHaveLength(1);
		expect(filter(nest(tree), 'linux')[0].folders[0].devices).toHaveLength(2);
		expect(filter(nest(tree), 'nothing matches')).toEqual([]);
		expect(filter(nest(tree), '  ')).toHaveLength(2);
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
