/**
 * The WebSocket to a device's terminal (crates/server/src/api/terminal.rs).
 * The server is the SSH client; this only carries keystrokes, output and
 * resize events.
 */

export interface Size {
	cols: number;
	rows: number;
}

export interface Credentials {
	username: string;
	password: string;
}

export type ServerEvent =
	| { type: 'connected'; host_key_fingerprint: string; pinned: boolean }
	| { type: 'closed'; exit_status: number | null }
	| { type: 'error'; code: string; params: Record<string, unknown> };

export interface Handlers {
	onevent: (event: ServerEvent) => void;
	onoutput: (data: Uint8Array) => void;
	/** The socket closed (after an event, or unexpectedly). */
	ondisconnect: () => void;
}

export interface TerminalConnection {
	send: (data: string) => void;
	resize: (size: Size) => void;
	close: () => void;
}

export function terminalUrl(deviceId: string, location: Location = window.location): string {
	const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
	return `${scheme}://${location.host}/api/devices/${encodeURIComponent(deviceId)}/terminal`;
}

/** Parses a text frame of the server; `null` for anything unexpected. */
export function parseEvent(text: string): ServerEvent | null {
	try {
		const event = JSON.parse(text) as { type?: unknown };
		return event.type === 'connected' || event.type === 'closed' || event.type === 'error'
			? (event as ServerEvent)
			: null;
	} catch {
		return null;
	}
}

export function connect(
	deviceId: string,
	size: Size,
	credentials: Credentials | null,
	handlers: Handlers
): TerminalConnection {
	const socket = new WebSocket(terminalUrl(deviceId));
	socket.binaryType = 'arraybuffer';
	const encoder = new TextEncoder();

	socket.onopen = () => {
		socket.send(JSON.stringify({ type: 'start', ...size, ...(credentials ?? {}) }));
	};
	socket.onmessage = (message) => {
		if (message.data instanceof ArrayBuffer) {
			handlers.onoutput(new Uint8Array(message.data));
		} else if (typeof message.data === 'string') {
			const event = parseEvent(message.data);
			if (event) handlers.onevent(event);
		}
	};
	socket.onclose = () => handlers.ondisconnect();

	return {
		send: (data) => {
			if (socket.readyState === WebSocket.OPEN) socket.send(encoder.encode(data));
		},
		resize: (next) => {
			if (socket.readyState === WebSocket.OPEN)
				socket.send(JSON.stringify({ type: 'resize', ...next }));
		},
		close: () => socket.close()
	};
}
