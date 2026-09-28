import { describe, expect, it, vi } from 'vitest';
import Guacamole from './guacamole/guacamole-common.js';
import {
	createTunnel,
	displayUrl,
	endsRegularly,
	failureOf,
	parseEvent,
	type ServerEvent
} from './tunnel';

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
		// Success: the session ended as it should (#221).
		expect(endsRegularly(0x0000)).toBe(true);
		expect(endsRegularly(0x020b)).toBe(false);
	});

	it('tells a session guacd ended from a lost connection', () => {
		class FakeSocket {
			static last: FakeSocket;
			readyState = 1;
			onopen?: () => void;
			onmessage?: (message: { data: string }) => void;
			onclose?: () => void;
			constructor() {
				FakeSocket.last = this;
			}
			send() {}
			close() {
				this.onclose?.();
			}
		}
		vi.stubGlobal('WebSocket', FakeSocket);
		vi.stubGlobal('window', { location: new URL('https://remotehub.example.com/') });
		/** A connected tunnel, its socket, and the events it reported. */
		const connected = () => {
			const events: ServerEvent['type'][] = [];
			const tunnel = createTunnel(
				'd',
				() => ({ width: 800, height: 600, dpi: 96 }),
				null,
				null,
				(event) => events.push(event.type)
			);
			tunnel.connect('');
			const socket = FakeSocket.last;
			socket.onmessage?.({ data: '{"type":"connected","certificate_fingerprint":null}' });
			socket.onmessage?.({ data: '4.sync,1.1;' });
			return { socket, events };
		};
		try {
			// Signed out on the device: guacd says goodbye before it closes.
			const signedOut = connected();
			signedOut.socket.onmessage?.({ data: '10.disconnect;' });
			expect(signedOut.events).toEqual(['connected', 'ended']);
			// A lost connection just closes.
			const lost = connected();
			lost.socket.close();
			expect(lost.events).toEqual(['connected']);
		} finally {
			vi.unstubAllGlobals();
		}
	});
});
