import { describe, expect, it } from 'vitest';
import { MAX_PICKS, frequent, rank, remember, type Pick, type Searchable } from './rank';

const NOW = Date.UTC(2026, 8, 25);
const DAY = 24 * 60 * 60 * 1000;

function item(name: string, description = '', host = ''): Searchable<string> {
	return {
		key: `device:${name}`,
		name,
		fields: [
			{ text: name, weight: 1 },
			{ text: host, weight: 0.8 },
			{ text: description, weight: 0.5 }
		],
		item: name
	};
}

const devices = [
	item('dc01', 'Domain controller Hamburg', 'dc01.corp.example'),
	item('dc02', 'Domain controller Bremen', 'dc02.corp.example'),
	item('fw-edge', 'Firewall am Büro', 'fw.corp.example'),
	item('core-sw1', 'Core switch', '10.20.0.2')
];

describe('matching', () => {
	it('finds parts of words in name, host and description', () => {
		expect(rank(devices, 'edg', [], NOW)).toEqual(['fw-edge']);
		expect(rank(devices, 'corp.ex', [], NOW)).toEqual(['dc01', 'dc02', 'fw-edge']);
		expect(rank(devices, 'contr', [], NOW)).toEqual(['dc01', 'dc02']);
	});

	it('ignores case and accents', () => {
		expect(rank(devices, 'BURO', [], NOW)).toEqual(['fw-edge']);
		expect(rank(devices, 'büro', [], NOW)).toEqual(['fw-edge']);
	});

	it('needs every word somewhere', () => {
		expect(rank(devices, 'controller brem', [], NOW)).toEqual(['dc02']);
		expect(rank(devices, 'controller firewall', [], NOW)).toEqual([]);
	});

	it('puts a match in the name before one in the description', () => {
		const items = [item('backup', 'runs the core jobs'), item('core-db')];
		expect(rank(items, 'core', [], NOW)).toEqual(['core-db', 'backup']);
	});

	it('finds nothing for an empty query', () => {
		expect(rank(devices, '  ', [], NOW)).toEqual([]);
	});
});

describe('learning', () => {
	const picked = (name: string, query: string, count = 1, daysAgo = 0): Pick => ({
		key: `device:${name}`,
		query,
		count,
		last: NOW - daysAgo * DAY
	});

	it('moves up what was picked for the same query', () => {
		expect(rank(devices, 'dc', [], NOW)).toEqual(['dc01', 'dc02']);
		expect(rank(devices, 'dc', [picked('dc02', 'dc')], NOW)).toEqual(['dc02', 'dc01']);
	});

	it('helps while the query is still being typed', () => {
		expect(rank(devices, 'd', [picked('dc02', 'dc02')], NOW)[0]).toBe('dc02');
	});

	it('never brings in what does not match', () => {
		expect(rank(devices, 'dc', [picked('core-sw1', 'dc', 50)], NOW)).toEqual(['dc01', 'dc02']);
	});

	it('forgets slowly: a recent pick beats an old one picked as often', () => {
		const picks = [picked('dc01', 'dc', 3, 90), picked('dc02', 'dc', 3, 1)];
		expect(rank(devices, 'dc', picks, NOW)).toEqual(['dc02', 'dc01']);
	});

	it('does not let one pick beat a far better match', () => {
		const items = [item('core-db'), item('backup', 'runs the core jobs')];
		const picks = [picked('backup', 'core')];
		expect(rank(items, 'core', picks, NOW)).toEqual(['core-db', 'backup']);
	});

	it('counts picks per item and query, and keeps the most useful', () => {
		let picks = remember([], 'device:dc01', ' DC ', NOW);
		picks = remember(picks, 'device:dc01', 'dc', NOW + 1);
		expect(picks).toEqual([{ key: 'device:dc01', query: 'dc', count: 2, last: NOW + 1 }]);

		const many = Array.from({ length: MAX_PICKS }, (_, i) => picked(`x${i}`, 'x', 1, 200));
		const trimmed = remember(many, 'device:dc01', 'dc', NOW);
		expect(trimmed).toHaveLength(MAX_PICKS);
		expect(trimmed.some((pick) => pick.key === 'device:dc01')).toBe(true);
	});

	it('names the favourites', () => {
		const picks = [picked('dc01', '', 1), picked('dc02', 'dc', 4), picked('dc02', '', 1)];
		expect(frequent(picks, NOW, 1)).toEqual(['device:dc02']);
	});
});
