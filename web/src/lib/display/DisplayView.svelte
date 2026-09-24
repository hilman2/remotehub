<script lang="ts">
	import Guacamole from './guacamole/guacamole-common.js';
	import { attachClipboard, isPasteKey, mayReadClipboard } from './clipboard';
	import { createTunnel, type Credentials, type ServerEvent } from './tunnel';
	import { m } from '$lib/paraglide/messages';

	let {
		deviceId,
		name,
		credentials,
		visible = true,
		onevent,
		onfailure,
		onend
	}: {
		deviceId: string;
		name: string;
		credentials: Credentials | null;
		/** Takes the keyboard whenever it is shown again. */
		visible?: boolean;
		onevent: (event: ServerEvent) => void;
		/** guacd ended the session with this Guacamole status. */
		onfailure: (code: number, detail: string) => void;
		onend: () => void;
	} = $props();

	let view: HTMLDivElement;

	$effect(() => {
		if (visible) view.focus();
	});

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
		// A hidden tab has no size (display: none). Its session keeps the size
		// it had and is fitted again when the tab is shown.
		const hidden = () => container.clientWidth === 0;
		const fit = () => {
			if (hidden()) return;
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
			if (hidden()) return;
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
		// The browser's clipboard needs a secure context (https or localhost).
		const clipboard = attachClipboard(client, navigator.clipboard ?? null);
		// Where the browser lets the page read the clipboard, the session gets
		// it when the view gains focus or is clicked, so the session's own
		// paste menus see what was copied outside. Elsewhere only a paste key
		// brings it. The tunnel drops what is sent before the session is up.
		let reads = false;
		let up = false;
		const pull = () => {
			if (!up || document.activeElement !== container) return;
			if (reads) void clipboard.pull();
			else void clipboard.flush();
		};
		void mayReadClipboard().then((allowed) => {
			reads = allowed;
			pull();
		});
		container.addEventListener('focus', pull);
		window.addEventListener('focus', pull);

		// Typing goes to the session once its picture was clicked.
		const focus = () => {
			container.focus();
			pull();
		};
		element.addEventListener('mousedown', focus);

		// A paste key is held back until the browser's paste event has put the
		// clipboard into the session: otherwise the session pastes what it
		// held before. Without a paste event (nothing to paste), it goes after
		// a moment.
		let heldKey: number | null = null;
		let holding: ReturnType<typeof setTimeout> | undefined;
		const release = () => {
			clearTimeout(holding);
			if (heldKey === null) return;
			client.sendKeyEvent(1, heldKey);
			heldKey = null;
		};
		const paste = (event: ClipboardEvent) => {
			event.preventDefault();
			clipboard.fromBrowser(event.clipboardData?.getData('text/plain') ?? '');
			release();
		};
		container.addEventListener('paste', paste);

		const keyboard = new Guacamole.Keyboard(container);
		keyboard.onkeydown = (keysym) => {
			// A key press is a user's action: Firefox and Safari write the
			// session's text to the clipboard now.
			void clipboard.flush();
			if (isPasteKey(keysym, keyboard.modifiers)) {
				release();
				heldKey = keysym;
				holding = setTimeout(release, 100);
				// The browser's paste happens only if the key is not cancelled.
				return true;
			}
			client.sendKeyEvent(1, keysym);
			// Keys go to the remote session, not to the browser.
			return false;
		};
		keyboard.onkeyup = (keysym) => {
			if (keysym === heldKey) release();
			client.sendKeyEvent(0, keysym);
		};

		client.onerror = (status) => onfailure(status.code, status.message);
		client.onstatechange = (state) => {
			up = state === Guacamole.Client.State.CONNECTED;
			pull();
			if (state === Guacamole.Client.State.DISCONNECTED) onend();
		};
		client.connect();
		container.focus();

		return () => {
			observer.disconnect();
			clearTimeout(resizing);
			clearTimeout(holding);
			element.removeEventListener('mousedown', focus);
			container.removeEventListener('focus', pull);
			container.removeEventListener('paste', paste);
			window.removeEventListener('focus', pull);
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
	bind:this={view}
	{@attach session}
	class="h-full w-full cursor-default overflow-hidden bg-[#07090c] outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-inset"
	role="application"
	aria-label={m.display_label({ name })}
	tabindex="0"
></div>
