import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CredentialInput, Tree } from '$lib/api/catalog';
import type { KdbxEntry } from './kdbx';
import { importInto } from './shared-kdbx';

/** What the import asked the server to create. */
const made: { parent: string | null; name: string }[] = [];
const credentials: { collection: string; name: string }[] = [];

vi.mock('$lib/api/catalog', async (original) => ({
	...(await original<typeof import('$lib/api/catalog')>()),
	createCollection: vi.fn(async (parent: string | null, name: string) => {
		made.push({ parent, name });
		return { ok: true, data: { id: `new-${name}` } };
	}),
	createCredential: vi.fn(async (input: CredentialInput) => {
		credentials.push({ collection: input.collection_id, name: input.name });
		return { ok: true, data: { id: `credential-${credentials.length}` } };
	})
}));
vi.mock('$lib/api/reveal', () => ({
	attachmentUrl: vi.fn(),
	reveal: vi.fn(),
	uploadAttachment: vi.fn()
}));

const entry = (path: string[], title: string): KdbxEntry => ({
	path,
	title,
	username: '',
	password: 'secret',
	url: '',
	notes: '',
	icon: 0,
	fields: [],
	files: []
});

const tree = {
	collections: [{ id: 'servers', parent_id: null, name: 'Servers', role: 'manage' }]
} as unknown as Tree;

beforeEach(() => {
	made.length = 0;
	credentials.length = 0;
});

describe('a KeePass file into Shared itself (#216)', () => {
	it('makes its groups collections at the top, and its top one after the file', async () => {
		const result = await importInto(
			tree,
			null,
			[entry([], 'Router'), entry(['Servers'], 'dc01'), entry(['Web', 'Shop'], 'Admin')],
			'Team'
		);
		expect(result).toMatchObject({ created: 3, failed: [] });
		// Servers exists already at the top: taken, not made again.
		expect(made).toEqual([
			{ parent: null, name: 'Team' },
			{ parent: null, name: 'Web' },
			{ parent: 'new-Web', name: 'Shop' }
		]);
		expect(credentials).toEqual([
			{ collection: 'new-Team', name: 'Router' },
			{ collection: 'servers', name: 'dc01' },
			{ collection: 'new-Shop', name: 'Admin' }
		]);
	});

	it('makes empty groups too, for a personal folder moved to Shared (#218)', async () => {
		const result = await importInto(tree, 'servers', [entry(['Home'], 'NAS')], '—', [
			['Home'],
			['Home', 'Empty']
		]);
		expect(result).toMatchObject({ created: 1, failed: [] });
		expect(made).toEqual([
			{ parent: 'servers', name: 'Home' },
			{ parent: 'new-Home', name: 'Empty' }
		]);
		expect(credentials).toEqual([{ collection: 'new-Home', name: 'NAS' }]);
	});
});
