<script lang="ts">
	import type { Protocol } from '$lib/api/catalog';
	import { PROTOCOL_SHORT } from './labels';

	/** The protocol as a short tag; RDP and VNC in the accent, SSH and HTTPS in their own colour. */
	let { protocol, large = false }: { protocol: Protocol; large?: boolean } = $props();

	const colour: Record<Protocol, string> = {
		rdp: 'text-accent',
		vnc: 'text-accent',
		ssh: 'text-ssh',
		https: 'text-web'
	};
</script>

<span
	class="inline-flex shrink-0 bg-surface-2 font-mono leading-tight font-medium {colour[protocol]}"
	class:chip-small={!large}
	class:chip-large={large}
>
	{PROTOCOL_SHORT[protocol]}
</span>

<style>
	.chip-small {
		border-radius: 5px;
		padding: 0.125rem 0.375rem;
		font-size: 10px;
	}

	.chip-large {
		border: 1px solid var(--line-strong);
		border-radius: 7px;
		padding: 0.375rem 0.625rem;
		font-size: 12px;
	}
</style>
