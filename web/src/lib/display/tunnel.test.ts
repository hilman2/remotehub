import { describe, expect, it } from 'vitest';
import Guacamole from './guacamole/guacamole-common.js';
import { displayUrl, failureOf, parseEvent } from './tunnel';

describe('display tunnel', () => {
	it('uses the secure WebSocket scheme on https', () => {
		const at = (href: string) => displayUrl('a b', new URL(href) as unknown as Location);
		expect(at('https://remotehub.example.com/connect/x')).toBe(
			'wss://remotehub.example.com/api/devices/a%20b/display'
		);
		expect(at('http://localhost:5180/')).toBe('ws://localhost:5180/api/devices/a%20b/display');
	});

	it('reads only the events it knows', () => {
		expect(parseEvent('{"type":"connected"}')).toEqual({ type: 'connected' });
		expect(parseEvent('{"type":"error","code":"forbidden","params":{}}')?.type).toBe('error');
		expect(parseEvent('{"type":"other"}')).toBeNull();
		expect(parseEvent('4.sync,1.0;')).toBeNull();
	});

	it('encodes instructions by code points, as the server parses them', () => {
		expect(Guacamole.Parser.toInstruction(['key', 'ä😀', 1])).toBe('3.key,2.ä😀,1.1;');
	});

	it('explains guacd status codes', () => {
		expect(failureOf(0x0207)).toBe('target');
		expect(failureOf(0x0208)).toBe('target');
		expect(failureOf(0x0301)).toBe('auth');
		expect(failureOf(0x020b)).toBe('ended');
		expect(failureOf(0x0200)).toBe('failed');
	});
});
