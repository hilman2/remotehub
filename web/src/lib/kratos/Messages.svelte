<script lang="ts">
	/** Kratos' messages of a flow, each with an icon for its kind. */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Info from '@lucide/svelte/icons/info';
	import { kratosText, type UiText } from './flow';

	let { texts }: { texts: UiText[] } = $props();
</script>

{#each texts as text, index (index)}
	<p class="flex items-start gap-2 text-sm" role={text.type === 'error' ? 'alert' : 'status'}>
		{#if text.type === 'error'}
			<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
		{:else if text.type === 'success'}
			<CircleCheck size={16} class="mt-0.5 shrink-0 text-ok" aria-hidden="true" />
		{:else}
			<Info size={16} class="mt-0.5 shrink-0 text-ink-3" aria-hidden="true" />
		{/if}
		{kratosText(text)}
	</p>
{/each}
