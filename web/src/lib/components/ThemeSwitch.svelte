<script lang="ts">
	import Monitor from '@lucide/svelte/icons/monitor';
	import Sun from '@lucide/svelte/icons/sun';
	import Moon from '@lucide/svelte/icons/moon';
	import { m } from '$lib/paraglide/messages';

	type Theme = 'system' | 'light' | 'dark';
	const STORAGE_KEY = 'remotehub-theme';

	function stored(): Theme {
		try {
			const value = localStorage.getItem(STORAGE_KEY);
			return value === 'light' || value === 'dark' ? value : 'system';
		} catch {
			return 'system';
		}
	}

	let theme = $state<Theme>(stored());

	$effect(() => {
		const html = document.documentElement;
		if (theme === 'system') delete html.dataset.theme;
		else html.dataset.theme = theme;
		try {
			if (theme === 'system') localStorage.removeItem(STORAGE_KEY);
			else localStorage.setItem(STORAGE_KEY, theme);
		} catch {
			// without storage the choice only lasts until the next reload
		}
	});

	const CHOICES = [
		{ value: 'system', label: m.theme_system, Icon: Monitor },
		{ value: 'light', label: m.theme_light, Icon: Sun },
		{ value: 'dark', label: m.theme_dark, Icon: Moon }
	] as const;
</script>

<div
	class="inline-flex rounded-lg border border-line bg-surface p-0.5"
	role="radiogroup"
	aria-label={m.theme_label()}
>
	{#each CHOICES as { value, label, Icon } (value)}
		<button
			type="button"
			role="radio"
			aria-checked={theme === value}
			title={label()}
			class="rounded-md p-1.5 text-ink-3 transition-colors hover:text-ink aria-checked:bg-surface-2 aria-checked:text-ink"
			onclick={() => (theme = value)}
		>
			<Icon size={15} aria-hidden="true" />
			<span class="sr-only">{label()}</span>
		</button>
	{/each}
</div>
