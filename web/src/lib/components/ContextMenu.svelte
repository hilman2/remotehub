<script module lang="ts">
	export interface ContextItem {
		label: string;
		/** The shortcut that does the same, shown beside the label. */
		keys?: string;
		/** Shown in the critical colour, besides its words. */
		danger?: boolean;
		disabled?: boolean;
		/** A line before this item. */
		separated?: boolean;
		onselect: () => void;
	}
</script>

<script lang="ts">
	/**
	 * A menu at the pointer, as a right click opens it (#193). Keys as in the
	 * WAI-ARIA menu pattern; Escape, a click elsewhere or a choice closes it.
	 */
	import { tick } from 'svelte';

	let {
		label,
		items,
		at,
		onclose
	}: {
		label: string;
		items: ContextItem[];
		/** Where it opens, in viewport pixels. */
		at: { x: number; y: number };
		onclose: () => void;
	} = $props();

	let menu = $state<HTMLDivElement>();
	let place = $state({ left: 0, top: 0 });

	const entries = () => [
		...(menu?.querySelectorAll<HTMLElement>('[role=menuitem]:not([disabled])') ?? [])
	];

	$effect(() => {
		void at;
		tick().then(() => {
			if (!menu) return;
			// Kept inside the window, as a desktop menu is.
			const { width, height } = menu.getBoundingClientRect();
			place = {
				left: Math.max(4, Math.min(at.x, window.innerWidth - width - 4)),
				top: Math.max(4, Math.min(at.y, window.innerHeight - height - 4))
			};
			entries()[0]?.focus();
		});
		const away = (event: PointerEvent) => {
			if (!menu?.contains(event.target as Node)) onclose();
		};
		document.addEventListener('pointerdown', away);
		return () => document.removeEventListener('pointerdown', away);
	});

	function onkeydown(event: KeyboardEvent) {
		const all = entries();
		const index = all.indexOf(document.activeElement as HTMLElement);
		if (event.key === 'Escape') {
			event.preventDefault();
			onclose();
		} else if (event.key === 'ArrowDown') {
			event.preventDefault();
			all[(index + 1) % all.length]?.focus();
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			all[(index - 1 + all.length) % all.length]?.focus();
		} else if (event.key === 'Tab') {
			onclose();
		}
	}

	function choose(item: ContextItem) {
		onclose();
		item.onselect();
	}
</script>

<div
	bind:this={menu}
	role="menu"
	aria-label={label}
	tabindex="-1"
	class="fixed z-50 min-w-60 rounded-xl border border-line-strong bg-surface p-1.5 text-sm shadow-lg"
	style:left="{place.left}px"
	style:top="{place.top}px"
	{onkeydown}
>
	{#each items as item (item.label)}
		{#if item.separated}
			<div role="separator" class="mx-1 my-1 h-px bg-line"></div>
		{/if}
		<button
			type="button"
			role="menuitem"
			class="flex w-full items-center gap-6 rounded-lg px-2.5 py-1.5 text-left hover:bg-surface-2 focus:bg-surface-2 focus:outline-none disabled:opacity-40 {item.danger
				? 'text-critical'
				: ''}"
			disabled={item.disabled}
			onclick={() => choose(item)}
		>
			<span class="flex-1">{item.label}</span>
			{#if item.keys}
				<kbd class="font-sans text-xs text-ink-3">{item.keys}</kbd>
			{/if}
		</button>
	{/each}
</div>
