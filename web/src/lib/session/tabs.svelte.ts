/**
 * The sessions open inside remotehub (#85), one tab each. They live as long
 * as the page does, across moves between its routes; a reload ends them.
 */
import type { Device } from '$lib/api/catalog';

/** How far a session got, for the icon on its tab: ended on its own, or failed. */
export type Phase = 'connecting' | 'connected' | 'closed' | 'failed';

export interface Tab {
	key: number;
	device: Device;
	phase: Phase;
}

class Tabs {
	list = $state<Tab[]>([]);
	/** The tab shown on the devices page; null shows the devices themselves. */
	active = $state<number | null>(null);
	#next = 1;

	/** Shows the device's session, and starts one if it has none yet. */
	open(device: Device) {
		const existing = this.list.find((tab) => tab.device.id === device.id);
		if (existing) {
			this.active = existing.key;
			return;
		}
		const key = this.#next++;
		this.list.push({ key, device, phase: 'connecting' });
		this.active = key;
	}

	/** Ends the session; if it was shown, its neighbour takes its place. */
	close(key: number) {
		const index = this.list.findIndex((tab) => tab.key === key);
		if (index < 0) return;
		this.list.splice(index, 1);
		if (this.active === key) {
			this.active = this.list[Math.min(index, this.list.length - 1)]?.key ?? null;
		}
	}

	setPhase(key: number, phase: Phase) {
		const tab = this.list.find((t) => t.key === key);
		if (tab) tab.phase = phase;
	}

	/** Ends every session, as signing out does. */
	clear() {
		this.list = [];
		this.active = null;
	}
}

export const tabs = new Tabs();
