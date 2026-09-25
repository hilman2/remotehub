import { describe, expect, it } from 'vitest';
import { withHistory, type EntryContent } from './vault';

const entry = (password: string, extra: Partial<EntryContent> = {}): EntryContent => ({
	title: 'Router',
	username: 'admin',
	password,
	url: '',
	notes: '',
	...extra
});

describe('the history of personal entries', () => {
	it('keeps what was before each change, newest first', () => {
		const second = withHistory(entry('one'), entry('two'));
		const third = withHistory(second, entry('three'));
		expect(third.history?.map((h) => h.password)).toEqual(['two', 'one']);
		// Earlier states carry no history of their own.
		expect(third.history?.[0]).not.toHaveProperty('history');
	});

	it('adds nothing for a save without a change', () => {
		const second = withHistory(entry('one'), entry('two'));
		expect(withHistory(second, entry('two')).history).toHaveLength(1);
	});

	it('keeps ten states, and fewer when the entry grows large', () => {
		let current = entry('0');
		for (let i = 1; i <= 12; i++) current = withHistory(current, entry(String(i)));
		expect(current.history).toHaveLength(10);
		const notes = 'x'.repeat(9000);
		let large = entry('a', { notes });
		for (let i = 0; i < 10; i++) large = withHistory(large, entry(`p${i}`, { notes }));
		expect(JSON.stringify(large).length).toBeLessThanOrEqual(60_000);
		expect(large.history!.length).toBeLessThan(10);
	});
});
