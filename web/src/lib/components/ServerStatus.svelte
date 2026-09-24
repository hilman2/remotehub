<script lang="ts">
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import CircleX from '@lucide/svelte/icons/circle-x';
	import DatabaseZap from '@lucide/svelte/icons/database-zap';
	import { fetchServerState, type ServerState } from '$lib/api/health';
	import { m } from '$lib/paraglide/messages';

	let state = $state<ServerState | null>(null);

	$effect(() => {
		const refresh = async () => (state = await fetchServerState());
		refresh();
		const timer = setInterval(refresh, 30_000);
		return () => clearInterval(timer);
	});
</script>

<!-- A state is never colour alone: always icon and text. -->
{#if state}
	<div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-3" role="status">
		{#if state.kind === 'ok'}
			<span class="inline-flex items-center gap-1.5">
				<CircleCheck size={14} class="text-ok" aria-hidden="true" />
				{m.server_ok()}
			</span>
		{:else if state.kind === 'database_unavailable'}
			<span class="inline-flex items-center gap-1.5 text-ink">
				<DatabaseZap size={14} class="text-critical" aria-hidden="true" />
				{m.server_database_unavailable()}
			</span>
		{:else}
			<span class="inline-flex items-center gap-1.5 text-ink">
				<CircleX size={14} class="text-critical" aria-hidden="true" />
				{m.server_unreachable()}
			</span>
		{/if}
		{#if state.kind !== 'unreachable'}
			<span>{m.footer_version({ version: state.version })}</span>
		{/if}
	</div>
{/if}
