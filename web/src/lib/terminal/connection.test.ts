import { describe, expect, it } from 'vitest';
import { parseEvent, terminalUrl } from './connection';

describe('terminalUrl', () => {
	it('uses wss behind https and ws otherwise', () => {
		const https = { protocol: 'https:', host: 'remotehub.example.com' } as Location;
		expect(terminalUrl('abc', https)).toBe('wss://remotehub.example.com/api/devices/abc/terminal');
		const http = { protocol: 'http:', host: 'localhost:5180' } as Location;
		expect(terminalUrl('a/b', http)).toBe('ws://localhost:5180/api/devices/a%2Fb/terminal');
	});
});

describe('parseEvent', () => {
	it('accepts the three server events', () => {
		expect(parseEvent('{"type":"closed","exit_status":0}')).toEqual({
			type: 'closed',
			exit_status: 0
		});
		expect(parseEvent('{"type":"error","code":"target_unreachable","params":{}}')?.type).toBe(
			'error'
		);
		expect(
			parseEvent('{"type":"connected","host_key_fingerprint":"SHA256:x","pinned":true}')
		).toMatchObject({ pinned: true });
	});

	it('ignores anything else', () => {
		expect(parseEvent('{"type":"surprise"}')).toBeNull();
		expect(parseEvent('not json')).toBeNull();
	});
});
