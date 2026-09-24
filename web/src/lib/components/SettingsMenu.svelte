<script module lang="ts">
	import type Settings from '@lucide/svelte/icons/settings';

	export interface MenuItem {
		label: string;
		icon?: typeof Settings;
		/** Shown in the critical colour, besides its words. */
		danger?: boolean;
		onselect: () => void;
	}
</script>

<script lang="ts">
	/**
	 * A gear that opens the actions rarely needed in daily use (#88). Shows
	 * nothing without items. Keys as in the WAI-ARIA menu button pattern.
	 */
	import SettingsIcon from '@lucide/svelte/icons/settings';

	let { label, items }: { label: string; items: MenuItem[] } = $props();

	const id = $props.id();
	let open = $state(false);
	let root = $state<HTMLDivElement>();
	let trigger = $state<HTMLButtonElement>();

	const entries = () => [...(root?.querySelectorAll<HTMLElement>('[role=menuitem]') ?? [])];

	$effect(() => {
		if (!open) return;
		entries()[0]?.focus();
		const away = (event: PointerEvent) => {
			if (!root?.contains(event.target as Node)) open = false;
		};
		document.addEventListener('pointerdown', away);
		return () => document.removeEventListener('pointerdown', away);
	});

	function onkeydown(event: KeyboardEvent) {
		const all = entries();
		const at = all.indexOf(document.activeElement as HTMLElement);
		const move: Record<string, number> = {
			ArrowDown: at + 1,
			ArrowUp: at - 1,
			Home: 0,
			End: all.length - 1
		};
		if (event.key in move) {
			event.preventDefault();
			all[(move[event.key] + all.length) % all.length]?.focus();
		} else if (event.key === 'Escape') {
			event.preventDefault();
			open = false;
			trigger?.focus();
		} else if (event.key === 'Tab') {
			open = false;
		}
	}

	function select(item: MenuItem) {
		open = false;
		item.onselect();
	}
</script>

{#if items.length > 0}
	<div class="relative" bind:this={root}>
		<button
			bind:this={trigger}
			type="button"
			class="flex size-10 items-center justify-center rounded-xl border border-line-strong text-ink-2 hover:bg-surface-2 hover:text-ink aria-expanded:bg-surface-2 aria-expanded:text-ink"
			title={label}
			aria-haspopup="menu"
			aria-expanded={open}
			aria-controls={open ? id : undefined}
			onclick={() => (open = !open)}
		>
			<SettingsIcon size={17} aria-hidden="true" />
			<span class="sr-only">{label}</span>
		</button>
		{#if open}
			<div
				{id}
				class="absolute top-full right-0 z-20 mt-2 flex min-w-48 flex-col rounded-xl border border-line-strong bg-surface p-1 shadow-lg"
				role="menu"
				aria-label={label}
				tabindex="-1"
				{onkeydown}
			>
				{#each items as item (item.label)}
					<button
						type="button"
						class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm outline-none hover:bg-surface-2 focus-visible:bg-surface-2 {item.danger
							? 'text-critical'
							: 'text-ink'}"
						role="menuitem"
						tabindex="-1"
						onclick={() => select(item)}
					>
						{#if item.icon}
							<item.icon size={15} class="shrink-0" aria-hidden="true" />
						{/if}
						{item.label}
					</button>
				{/each}
			</div>
		{/if}
	</div>
{/if}
