/**
 * What the footer says about the session on screen (#85): how far it got and,
 * once connected, the pinned host key or certificate. Only the session shown
 * writes here.
 */
import type { Phase } from './tabs.svelte';

export interface StatusLine {
	phase: Phase;
	text: string;
}

class Shown {
	line = $state<StatusLine | null>(null);
	#owner = 0;
	#next = 1;

	/** A number for one session view, to tell its writes from the others'. */
	claim() {
		return this.#next++;
	}

	set(owner: number, line: StatusLine) {
		this.#owner = owner;
		this.line = line;
	}

	/** Clears the line, if it is still the one `owner` wrote. */
	release(owner: number) {
		if (this.#owner !== owner) return;
		this.#owner = 0;
		this.line = null;
	}
}

export const shown = new Shown();
