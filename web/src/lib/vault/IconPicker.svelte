<script lang="ts">
	/** Chooses one of KeePass' standard icons (#98). */
	import { m } from '$lib/paraglide/messages';
	import { ICONS } from './icons';

	let { id, value = $bindable(0) }: { id: string; value?: number } = $props();
</script>

<div
	{id}
	class="mt-1 grid max-h-32 grid-cols-12 gap-1 overflow-y-auto rounded-lg border border-line bg-page p-1"
	role="radiogroup"
	aria-label={m.vault_icon()}
>
	{#each ICONS as Icon, number (number)}
		<button
			type="button"
			role="radio"
			aria-checked={value === number}
			aria-label={m.vault_icon_number({ number })}
			class="flex size-7 items-center justify-center rounded-md text-ink-2 hover:bg-surface-2 aria-checked:bg-accent aria-checked:text-accent-ink"
			onclick={() => (value = number)}
		>
			<Icon size={15} />
		</button>
	{/each}
</div>
