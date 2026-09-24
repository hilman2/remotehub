<script lang="ts">
	import Guacamole from './guacamole/guacamole-common.js';
	import { createTunnel, type Credentials, type ServerEvent } from './tunnel';
	import { m } from '$lib/paraglide/messages';

	let {
		deviceId,
		name,
		credentials,
		onevent,
		onfailure,
		onend
	}: {
		deviceId: string;
		name: string;
		credentials: Credentials | null;
		onevent: (event: ServerEvent) => void;
		/** guacd ended the session with this Guacamole status. */
		onfailure: (code: number, detail: string) => void;
		onend: () => void;
	} = $props();

	/** Opens the session into `container` and closes it when the view goes. */
	function session(container: HTMLDivElement) {
		const size = () => ({
			width: Math.max(320, Math.floor(container.clientWidth)),
			height: Math.max(200, Math.floor(container.clientHeight))
		});
		const tunnel = createTunnel(
			deviceId,
			() => ({ ...size(), dpi: 96, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone }),
			credentials,
			onevent
		);
		const client = new Guacamole.Client(tunnel);
		const display = client.getDisplay();
		const element = display.getElement();
		container.appendChild(element);

		// The remote picture fits into the window, never larger than it is.
		const fit = () => {
			const width = display.getWidth();
			const height = display.getHeight();
			if (width > 0 && height > 0) {
				display.scale(Math.min(container.clientWidth / width, container.clientHeight / height, 1));
			}
		};
		display.onresize = fit;
		// RDP follows the window's size; VNC keeps its own and is scaled.
		let resizing: ReturnType<typeof setTimeout> | undefined;
		const observer = new ResizeObserver(() => {
			fit();
			clearTimeout(resizing);
			resizing = setTimeout(() => {
				const { width, height } = size();
				client.sendSize(width, height);
			}, 300);
		});
		observer.observe(container);

		const mouse = new Guacamole.Mouse(element);
		mouse.onEach(['mousedown', 'mouseup', 'mousemove', 'mousewheel'], (event) =>
			client.sendMouseState(event.state, true)
		);
		// Typing goes to the session once its picture was clicked.
		const focus = () => container.focus();
		element.addEventListener('mousedown', focus);
		const keyboard = new Guacamole.Keyboard(container);
		keyboard.onkeydown = (keysym) => {
			client.sendKeyEvent(1, keysym);
			// Keys go to the remote session, not to the browser.
			return false;
		};
		keyboard.onkeyup = (keysym) => client.sendKeyEvent(0, keysym);

		client.onerror = (status) => onfailure(status.code, status.message);
		client.onstatechange = (state) => {
			if (state === Guacamole.Client.State.DISCONNECTED) onend();
		};
		client.connect();
		container.focus();

		return () => {
			observer.disconnect();
			clearTimeout(resizing);
			element.removeEventListener('mousedown', focus);
			keyboard.reset();
			client.onstatechange = null;
			client.disconnect();
			element.remove();
		};
	}
</script>

<!-- A remote desktop is one interactive widget that takes the keyboard:
     role application with tabindex, as WAI-ARIA describes it. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
	{@attach session}
	class="h-full w-full cursor-default overflow-hidden rounded-card bg-black outline-none focus-visible:ring-2 focus-visible:ring-accent"
	role="application"
	aria-label={m.display_label({ name })}
	tabindex="0"
></div>
