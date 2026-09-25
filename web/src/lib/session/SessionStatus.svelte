<script lang="ts">
	/** The footer's line about the session on screen, on one line. */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import CircleMinus from '@lucide/svelte/icons/circle-minus';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import { shown } from './status.svelte';
</script>

{#if shown.line}
	{@const { phase, text } = shown.line}
	<p class="flex min-w-0 items-center gap-1.5 text-xs text-ink-3" role="status" title={text}>
		{#if phase === 'connecting'}
			<LoaderCircle size={14} class="shrink-0 animate-spin" aria-hidden="true" />
		{:else if phase === 'connected'}
			<CircleCheck size={14} class="shrink-0 text-ok" aria-hidden="true" />
		{:else if phase === 'closed'}
			<CircleMinus size={14} class="shrink-0" aria-hidden="true" />
		{:else}
			<CircleAlert size={14} class="shrink-0 text-critical" aria-hidden="true" />
		{/if}
		<span class="truncate">{text}</span>
	</p>
{/if}
