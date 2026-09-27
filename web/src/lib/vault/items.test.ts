import { describe, expect, it } from 'vitest';
import type { Credential, Tree } from '$lib/api/catalog';
import {
	counter,
	listed,
	outline,
	personalFolders,
	personalItems,
	sharedItems,
	type Scope,
	type Sort
} from './items';
import type { Entry } from './vault';

function credential(id: string, name: string, collection: string, more: Partial<Credential> = {}) {
	return {
		id,
		collection_id: collection,
		name,
		username: 'admin',
		domain: '',
		version: 1,
		url: '',
		notes: '',
		icon: 0,
		fields: [],
		attachments: [],
		role: 'reveal',
		...more
	} satisfies Credential;
}

const tree: Tree = {
	may_create_top_level: false,
	open: [],
	purpose_required: false,
	folders: [],
	devices: [],
	profiles: [],
	collections: [
		{ id: 'm', parent_id: null, name: 'Müller', role: 'edit' },
		{ id: 'ms', parent_id: 'm', name: 'Server', role: 'edit' },
		{ id: 'b', parent_id: null, name: 'buying', role: 'reveal' }
	],
	credentials: [
		credential('c1', 'Domain admin', 'ms', { domain: 'MUELLER' }),
		credential('c2', 'Deploy portal', 'm'),
		credential('c3', 'Office shop', 'b', {
			fields: [{ name: 'TOTP', protected: true }],
			attachments: [{ id: 'f', name: 'invoice.pdf', size: 10 }]
		})
	]
};

const entry = (id: string, title: string, parent: string | null, folder = false): Entry => ({
	id,
	content: {
		...(folder ? { kind: 'folder' as const } : {}),
		parent,
		title,
		username: 'me',
		password: 'secret',
		url: '',
		notes: ''
	}
});

const entries: Entry[] = [
	entry('shops', 'Shops', null, true),
	entry('books', 'Books', 'shops', true),
	entry('p1', 'Bookshop', 'books'),
	entry('p2', 'Bank', null),
	{ id: 'broken', content: null }
];

const items = [...sharedItems(tree), ...personalItems(entries)];
const context = {
	folders: personalFolders(entries),
	collections: tree.collections,
	picks: [],
	now: 0
};
const titles = (scope: Scope, query = '', sort: Sort = 'name') =>
	listed(items, { ...context, scope, sort, query, locale: 'en' }).map((item) => item.title);

describe('the vault list', () => {
	it('holds shared and readable personal entries, never folders', () => {
		expect(titles({ kind: 'all' })).toEqual([
			'Bank',
			'Bookshop',
			'Deploy portal',
			'Domain admin',
			'Office shop'
		]);
	});

	it('writes the path and the domain as the list shows them', () => {
		const admin = items.find((item) => item.id === 'c1');
		expect(admin?.where).toBe('Müller / Server');
		expect(admin?.username).toBe('MUELLER\\admin');
		expect(items.find((item) => item.id === 'p1')?.where).toBe('Shops / Books');
	});

	it('reaches into what lies below a collection or personal folder, and no further', () => {
		expect(titles({ kind: 'collection', id: 'm' })).toEqual(['Deploy portal', 'Domain admin']);
		expect(titles({ kind: 'collection', id: 'ms' })).toEqual(['Domain admin']);
		expect(titles({ kind: 'personal', folder: 'shops' })).toEqual(['Bookshop']);
		expect(titles({ kind: 'personal', folder: null })).toEqual(['Bank', 'Bookshop']);
	});

	it('filters by kind', () => {
		expect(titles({ kind: 'filter', filter: 'totp' })).toEqual(['Office shop']);
		expect(titles({ kind: 'filter', filter: 'files' })).toEqual(['Office shop']);
	});

	it('searches the whole vault, whatever the scope', () => {
		expect(titles({ kind: 'collection', id: 'b' }, 'book')).toEqual(['Bookshop']);
		expect(titles({ kind: 'personal', folder: null }, 'server')).toEqual(['Domain admin']);
	});

	it('sorts by use, and by place with the personal vault first', () => {
		const picks = [
			{ key: 'credential:c3', query: '', count: 5, last: 0 },
			{ key: 'p2', query: '', count: 2, last: 0 }
		];
		const used = listed(items, {
			...context,
			picks,
			scope: { kind: 'all' },
			sort: 'used',
			query: '',
			locale: 'en'
		});
		expect(used.map((item) => item.title).slice(0, 2)).toEqual(['Office shop', 'Bank']);
		expect(titles({ kind: 'all' }, '', 'place')).toEqual([
			'Bank',
			'Bookshop',
			'Office shop',
			'Deploy portal',
			'Domain admin'
		]);
	});

	it('counts what each scope holds', () => {
		const count = counter(items, context);
		expect(count({ kind: 'all' })).toBe(5);
		expect(count({ kind: 'collection', id: 'm' })).toBe(2);
		expect(count({ kind: 'personal', folder: 'books' })).toBe(1);
	});
});

describe('the outline', () => {
	it('puts each node right before what lies below it, sorted by name', () => {
		expect(outline(tree.collections, 'en')).toEqual([
			{ id: 'b', name: 'buying', depth: 0 },
			{ id: 'm', name: 'Müller', depth: 0 },
			{ id: 'ms', name: 'Server', depth: 1 }
		]);
	});

	it('shows a node whose parent is not visible at the top', () => {
		expect(outline([{ id: 'x', parent_id: 'hidden', name: 'Lonely' }])).toEqual([
			{ id: 'x', name: 'Lonely', depth: 0 }
		]);
	});
});
