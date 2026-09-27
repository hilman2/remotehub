import { describe, expect, it } from 'vitest';
import type { Credential, Tree } from '$lib/api/catalog';
import {
	counter,
	isExpired,
	isExpiring,
	listed,
	outline,
	personalFolders,
	personalItems,
	sharedItems,
	type Column,
	type Scope
} from './items';
import type { Entry, EntryContent } from './vault';

function credential(id: string, name: string, collection: string, more: Partial<Credential> = {}) {
	return {
		id,
		collection_id: collection,
		name,
		username: 'admin',
		version: 1,
		url: '',
		notes: '',
		icon: 0,
		fields: [],
		attachments: [],
		tags: [],
		expires_on: null,
		has_totp: false,
		updated_at: '2026-09-01T10:00:00Z',
		deleted_at: null,
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
		credential('c1', 'Domain admin', 'ms', {
			username: 'MUELLER\\admin',
			updated_at: '2026-09-20T08:00:00Z',
			tags: ['windows']
		}),
		credential('c2', 'Deploy portal', 'm', { expires_on: '2026-09-30' }),
		credential('c3', 'Office shop', 'b', {
			has_totp: true,
			attachments: [{ id: 'f', name: 'invoice.pdf', size: 10 }],
			expires_on: '2026-09-01'
		}),
		credential('c4', 'Old supplier', 'b', { deleted_at: '2026-09-25T12:00:00Z' })
	]
};

const entry = (
	id: string,
	title: string,
	parent: string | null,
	more: Partial<EntryContent> = {}
): Entry => ({
	id,
	content: { parent, title, username: 'me', password: 'secret', url: '', notes: '', ...more }
});

const entries: Entry[] = [
	entry('shops', 'Shops', null, { kind: 'folder' }),
	entry('books', 'Books', 'shops', { kind: 'folder' }),
	entry('p1', 'Bookshop', 'books', { changed: '2026-09-10T00:00:00Z', notes: 'Paperbacks' }),
	entry('p2', 'Bank', null, {
		fields: [{ name: 'otp', value: 'otpauth://totp/x?secret=JBSWY3DPEHPK3PXP', protected: true }]
	}),
	entry('p3', 'Forum', null, { deleted: '2026-09-26T00:00:00Z' }),
	{ id: 'broken', content: null }
];

const TODAY = '2026-09-27';
const items = [...sharedItems(tree), ...personalItems(entries)];
const context = {
	folders: personalFolders(entries),
	collections: tree.collections,
	picks: [],
	now: 0,
	today: TODAY
};
const titles = (scope: Scope, query = '', column: Column = 'title', descending = false) =>
	listed(items, {
		...context,
		scope,
		sort: { column, descending },
		query,
		locale: 'en'
	}).map((item) => item.title);

describe('the vault table', () => {
	it('holds shared and readable personal entries, never folders, the bins apart', () => {
		expect(titles({ kind: 'all' })).toEqual([
			'Bank',
			'Bookshop',
			'Deploy portal',
			'Domain admin',
			'Office shop'
		]);
		expect(titles({ kind: 'bin', side: 'shared' })).toEqual(['Old supplier']);
		expect(titles({ kind: 'bin', side: 'personal' })).toEqual(['Forum']);
	});

	it('keeps the user name as written and the path of the collection', () => {
		const admin = items.find((item) => item.id === 'c1');
		expect(admin?.where).toBe('Müller / Server');
		expect(admin?.username).toBe('MUELLER\\admin');
		expect(items.find((item) => item.id === 'p1')?.where).toBe('Shops / Books');
	});

	it('reaches into what lies below a collection or personal folder, and no further', () => {
		expect(titles({ kind: 'collection', id: 'm' })).toEqual(['Deploy portal', 'Domain admin']);
		expect(titles({ kind: 'collection', id: 'ms' })).toEqual(['Domain admin']);
		expect(titles({ kind: 'collection', id: 'b' })).toEqual(['Office shop']);
		expect(titles({ kind: 'personal', folder: 'shops' })).toEqual(['Bookshop']);
		expect(titles({ kind: 'personal', folder: null })).toEqual(['Bank', 'Bookshop']);
	});

	it('finds what runs out: run out today or before, and within two weeks', () => {
		expect(titles({ kind: 'expiring' })).toEqual(['Deploy portal', 'Office shop']);
		const [portal, shop] = ['c2', 'c3'].map((id) => items.find((item) => item.id === id)!);
		expect(isExpired(shop, TODAY)).toBe(true);
		expect(isExpired(portal, TODAY)).toBe(false);
		expect(isExpiring(portal, TODAY)).toBe(true);
		expect(isExpiring(portal, '2026-09-01')).toBe(false);
	});

	it('knows one-time passwords, the shared ones and those kept in a field', () => {
		const withTotp = items.filter((item) => item.hasTotp).map((item) => item.title);
		expect(withTotp.sort()).toEqual(['Bank', 'Office shop']);
	});

	it('searches the whole vault but the bins, tags and notes included', () => {
		expect(titles({ kind: 'collection', id: 'b' }, 'book')).toEqual(['Bookshop']);
		expect(titles({ kind: 'personal', folder: null }, 'windows')).toEqual(['Domain admin']);
		expect(titles({ kind: 'all' }, 'paperbacks')).toEqual(['Bookshop']);
		expect(titles({ kind: 'all' }, 'supplier')).toEqual([]);
	});

	it('sorts by any column, either way, ties by title', () => {
		expect(titles({ kind: 'all' }, '', 'title', true)).toEqual([
			'Office shop',
			'Domain admin',
			'Deploy portal',
			'Bookshop',
			'Bank'
		]);
		// Newest first; an entry never saved since #193 has no time and comes last.
		expect(titles({ kind: 'all' }, '', 'changed', true).slice(0, 2)).toEqual([
			'Domain admin',
			'Bookshop'
		]);
		expect(titles({ kind: 'all' }, '', 'username')[0]).toBe('Deploy portal');
	});

	it('shows recent entries in the order of use', () => {
		const picks = [
			{ key: 'credential:c3', query: '', count: 5, last: 0 },
			{ key: 'p2', query: '', count: 2, last: 0 }
		];
		const recent = listed(items, {
			...context,
			picks,
			scope: { kind: 'recent' },
			sort: { column: 'title', descending: false },
			query: ''
		});
		expect(recent.map((item) => item.title)).toEqual(['Office shop', 'Bank']);
	});

	it('counts what each scope holds', () => {
		const count = counter(items, context);
		expect(count({ kind: 'all' })).toBe(5);
		expect(count({ kind: 'collection', id: 'm' })).toBe(2);
		expect(count({ kind: 'personal', folder: 'books' })).toBe(1);
		expect(count({ kind: 'bin', side: 'shared' })).toBe(1);
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
