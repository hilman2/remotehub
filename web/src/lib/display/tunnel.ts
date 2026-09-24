/**
 * The WebSocket to a device's display (crates/server/src/api/display.rs) as a
 * Guacamole tunnel. The server opens RDP or VNC through guacd with the
 * credentials; the browser sends a start frame, then only exchanges
 * Guacamole instructions for input and drawing.
 */
import Guacamole from './guacamole/guacamole-common.js';

export interface Credentials {
	username: string;
	password: string;
}

export interface Start {
	width: number;
	height: number;
	dpi: number;
	timezone?: string;
}

export type ServerEvent =
	{ type: 'connected' } | { type: 'error'; code: string; params: Record<string, unknown> };

export function displayUrl(deviceId: string, location: Location = window.location): string {
	const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
	return `${scheme}://${location.host}/api/devices/${encodeURIComponent(deviceId)}/display`;
}

/** Parses the server's JSON frame before the connection is up; `null` otherwise. */
export function parseEvent(text: string): ServerEvent | null {
	try {
		const event = JSON.parse(text) as { type?: unknown };
		return event.type === 'connected' || event.type === 'error' ? (event as ServerEvent) : null;
	} catch {
		return null;
	}
}

/**
 * A tunnel for Guacamole.Client over the display WebSocket. `start` is asked
 * when the socket opens; `onevent` hears the server's JSON frames.
 */
export function createTunnel(
	deviceId: string,
	start: () => Start,
	credentials: Credentials | null,
	onevent: (event: ServerEvent) => void
): Guacamole.Tunnel {
	const tunnel = new Guacamole.Tunnel();
	const State = Guacamole.Tunnel.State;
	let socket: WebSocket | null = null;
	let connected = false;

	tunnel.connect = () => {
		tunnel.setState(State.CONNECTING);
		const parser = new Guacamole.Parser();
		parser.oninstruction = (opcode, args) => tunnel.oninstruction?.(opcode, args);

		socket = new WebSocket(displayUrl(deviceId));
		socket.onopen = () => {
			socket?.send(JSON.stringify({ type: 'start', ...start(), ...(credentials ?? {}) }));
		};
		socket.onmessage = (message) => {
			if (typeof message.data !== 'string') return;
			if (!connected) {
				const event = parseEvent(message.data);
				if (event?.type === 'connected') {
					connected = true;
					tunnel.setState(State.OPEN);
				}
				if (event) onevent(event);
				return;
			}
			try {
				parser.receive(message.data);
			} catch (error) {
				tunnel.onerror?.(new Guacamole.Status(Guacamole.Status.Code.SERVER_ERROR, String(error)));
				socket?.close();
			}
		};
		socket.onclose = () => tunnel.setState(State.CLOSED);
	};

	tunnel.disconnect = () => {
		socket?.close();
		tunnel.setState(State.CLOSED);
	};

	tunnel.sendMessage = (...elements: unknown[]) => {
		if (connected && socket?.readyState === WebSocket.OPEN && elements.length > 0) {
			socket.send(Guacamole.Parser.toInstruction(elements));
		}
	};

	return tunnel;
}

/** What went wrong, by Guacamole status code, in the words of the messages. */
export type Failure = 'target' | 'auth' | 'ended' | 'failed';

export function failureOf(code: number): Failure {
	// 0x0202 upstream timeout, 0x0203 upstream error, 0x0207 upstream not
	// found, 0x0208 upstream unavailable: guacd could not reach or open the
	// target (unreachable, refused, unexpected certificate).
	if ([0x0202, 0x0203, 0x0207, 0x0208].includes(code)) return 'target';
	// 0x0301 unauthorized, 0x0303 forbidden.
	if (code === 0x0301 || code === 0x0303) return 'auth';
	// 0x0209 session conflict, 0x020a session timeout, 0x020b session closed.
	if ([0x0209, 0x020a, 0x020b].includes(code)) return 'ended';
	return 'failed';
}
