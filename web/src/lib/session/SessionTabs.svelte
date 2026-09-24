<script lang="ts">
	/** The tabs of the open sessions, above every page while there are any. */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import CircleMinus from '@lucide/svelte/icons/circle-minus';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import X from '@lucide/svelte/icons/x';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import ProtocolChip from '$lib/catalog/ProtocolChip.svelte';
	import { m } from '$lib/paraglide/messages';
	import { tabs, type Phase } from './tabs.svelte';

	const PHASES: Record<Phase, () => string> = {
		connecting: m.session_connecting,
		connected: m.session_connected,
		closed: m.session_closed,
		failed: m.session_failed
	};

	const onDevices = $derived(page.url.pathname === resolve('/'));

	/** Shows the session; the sessions are shown on the devices page. */
	function show(key: number) {
		tabs.active = key;
		if (!onDevices) goto(resolve('/'));
	}
</script>

<nav class="border-b border-line bg-sunken" aria-label={m.session_tabs()}>
	<ul class="flex gap-1 overflow-x-auto px-3 pt-1.5">
		{#each tabs.list as tab (tab.key)}
			{@const current = onDevices && tabs.active === tab.key}
			<li
				class="group flex max-w-64 shrink-0 items-center rounded-t-lg border border-b-0 text-sm {current
					? 'border-line-strong bg-page text-ink'
					: 'border-transparent text-ink-2 hover:bg-surface-2 hover:text-ink'}"
			>
				<button
					type="button"
					class="flex min-w-0 items-center gap-2 py-2 pr-1 pl-3"
					aria-current={current ? 'page' : undefined}
					title="{tab.device.name} · {PHASES[tab.phase]()}"
					onclick={() => show(tab.key)}
				>
					{#if tab.phase === 'connecting'}
						<LoaderCircle size={14} class="shrink-0 animate-spin text-ink-3" aria-hidden="true" />
					{:else if tab.phase === 'connected'}
						<CircleCheck size={14} class="shrink-0 text-ok" aria-hidden="true" />
					{:else if tab.phase === 'closed'}
						<CircleMinus size={14} class="shrink-0 text-ink-3" aria-hidden="true" />
					{:else}
						<CircleAlert size={14} class="shrink-0 text-critical" aria-hidden="true" />
					{/if}
					<ProtocolChip protocol={tab.device.protocol} />
					<span class="truncate">{tab.device.name}</span>
					<span class="sr-only">{PHASES[tab.phase]()}</span>
				</button>
				<button
					type="button"
					class="mr-1.5 rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
					title={m.session_close({ name: tab.device.name })}
					onclick={() => tabs.close(tab.key)}
				>
					<X size={14} aria-hidden="true" />
					<span class="sr-only">{m.session_close({ name: tab.device.name })}</span>
				</button>
			</li>
		{/each}
	</ul>
</nav>
