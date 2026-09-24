<script lang="ts">
	import type { Snippet } from 'svelte';
	import X from '@lucide/svelte/icons/x';
	import { m } from '$lib/paraglide/messages';

	let {
		open = $bindable(false),
		title,
		children
	}: { open: boolean; title: string; children: Snippet } = $props();

	let dialog: HTMLDialogElement;

	// The native modal dialog: focus trap, Escape and backdrop come for free.
	$effect(() => {
		if (open && !dialog.open) dialog.showModal();
		if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	onclose={() => (open = false)}
	class="m-auto w-full max-w-lg rounded-card border border-line bg-surface p-0 text-ink shadow-float backdrop:bg-black/40"
>
	<div class="flex items-center justify-between border-b border-line px-5 py-3">
		<h2 class="font-semibold">{title}</h2>
		<button
			type="button"
			class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
			title={m.action_close()}
			onclick={() => (open = false)}
		>
			<X size={16} aria-hidden="true" />
			<span class="sr-only">{m.action_close()}</span>
		</button>
	</div>
	<div class="px-5 py-4">
		{@render children()}
	</div>
</dialog>
