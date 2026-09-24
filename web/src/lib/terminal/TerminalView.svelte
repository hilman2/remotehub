<script lang="ts">
	import { FitAddon } from '@xterm/addon-fit';
	import { Terminal } from '@xterm/xterm';
	import '@xterm/xterm/css/xterm.css';
	import { connect, type Credentials, type ServerEvent } from './connection';

	let {
		deviceId,
		credentials,
		onevent,
		onend
	}: {
		deviceId: string;
		credentials: Credentials | null;
		onevent: (event: ServerEvent) => void;
		onend: () => void;
	} = $props();

	let container: HTMLDivElement;

	// One terminal and one connection per mount; the page remounts to reconnect.
	$effect(() => {
		const terminal = new Terminal({
			cursorBlink: true,
			fontFamily: "'JetBrains Mono Variable', ui-monospace, Consolas, monospace",
			fontSize: 14,
			lineHeight: 1.25,
			scrollback: 5000,
			// Night Ops in both themes: a terminal stays dark.
			theme: {
				background: '#07090c',
				foreground: '#c9d1db',
				cursor: '#5eead4',
				cursorAccent: '#04201b',
				selectionBackground: '#1e3a36'
			}
		});
		const fit = new FitAddon();
		terminal.loadAddon(fit);
		terminal.open(container);
		fit.fit();

		const connection = connect(
			deviceId,
			{ cols: terminal.cols, rows: terminal.rows },
			credentials,
			{
				onoutput: (data) => terminal.write(data),
				onevent,
				ondisconnect: onend
			}
		);
		const input = terminal.onData((data) => connection.send(data));
		const observer = new ResizeObserver(() => {
			fit.fit();
			connection.resize({ cols: terminal.cols, rows: terminal.rows });
		});
		observer.observe(container);
		terminal.focus();

		return () => {
			observer.disconnect();
			input.dispose();
			connection.close();
			terminal.dispose();
		};
	});
</script>

<div bind:this={container} class="h-full w-full overflow-hidden bg-[#07090c] p-3"></div>
