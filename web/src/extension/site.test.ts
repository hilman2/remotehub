import { describe, expect, it } from 'vitest';
import cases from './site-cases.json';
import { matches, pageHost } from './site';

describe('which pages a login belongs to', () => {
	it.each(cases)('$login on $page: $why', ({ login, page, matches: expected }) => {
		expect(matches(login, page)).toBe(expected);
	});
});

describe('the host in the popup', () => {
	it('names web pages only', () => {
		expect(pageHost('https://login.example.com:8443/path?q')).toBe('login.example.com:8443');
		expect(pageHost('chrome://extensions')).toBeNull();
		expect(pageHost('about:blank')).toBeNull();
		expect(pageHost(undefined)).toBeNull();
	});
});
