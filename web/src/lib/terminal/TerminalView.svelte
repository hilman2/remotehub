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
			fontFamily: "'Cascadia Mono', 'JetBrains Mono', ui-monospace, Consolas, monospace",
			fontSize: 14,
			scrollback: 5000,
			theme: { background: '#0d0d0d', foreground: '#e8e8e3', cursor: '#3987e5' }
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

<div bind:this={container} class="h-full w-full overflow-hidden rounded-lg bg-[#0d0d0d] p-2"></div>
