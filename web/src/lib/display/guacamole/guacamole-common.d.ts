/**
 * Types for the parts of guacamole-common-js 1.6.0 that remotehub uses
 * (guacamole-common.js next to this file).
 */

declare namespace Guacamole {
	class Status {
		constructor(code: number, message?: string);
		code: number;
		message: string;
		isError(): boolean;
		static Code: Record<string, number>;
	}

	class Tunnel {
		state: number;
		uuid: string | null;
		connect(data?: string): void;
		disconnect(): void;
		sendMessage(...elements: unknown[]): void;
		setState(state: number): void;
		isConnected(): boolean;
		oninstruction: ((opcode: string, args: string[]) => void) | null;
		onstatechange: ((state: number) => void) | null;
		onerror: ((status: Status) => void) | null;
		static State: { CONNECTING: number; OPEN: number; CLOSED: number; UNSTABLE: number };
		static INTERNAL_DATA_OPCODE: string;
	}

	class Parser {
		receive(packet: string, isBuffer?: boolean): void;
		oninstruction: ((opcode: string, args: string[]) => void) | null;
		static toInstruction(elements: ArrayLike<unknown>): string;
	}

	class Display {
		getElement(): HTMLElement;
		getWidth(): number;
		getHeight(): number;
		getScale(): number;
		scale(scale: number): void;
		showCursor(shown: boolean): void;
		onresize: ((width: number, height: number) => void) | null;
	}

	namespace Mouse {
		class State {
			x: number;
			y: number;
			left: boolean;
			middle: boolean;
			right: boolean;
			up: boolean;
			down: boolean;
		}
		class Event {
			state: State;
		}
	}

	class Mouse {
		constructor(element: HTMLElement);
		onEach(types: string[], listener: (event: Mouse.Event) => void): void;
	}

	class Keyboard {
		constructor(element?: HTMLElement | Document);
		onkeydown: ((keysym: number) => boolean | void) | null;
		onkeyup: ((keysym: number) => void) | null;
		reset(): void;
	}

	class Client {
		constructor(tunnel: Tunnel);
		getDisplay(): Display;
		connect(data?: string): void;
		disconnect(): void;
		sendSize(width: number, height: number): void;
		sendKeyEvent(pressed: 0 | 1 | boolean, keysym: number): void;
		sendMouseState(state: Mouse.State, applyDisplayScale?: boolean): void;
		onstatechange: ((state: number) => void) | null;
		onerror: ((status: Status) => void) | null;
		static State: {
			IDLE: number;
			CONNECTING: number;
			WAITING: number;
			CONNECTED: number;
			DISCONNECTING: number;
			DISCONNECTED: number;
		};
	}
}

export default Guacamole;
