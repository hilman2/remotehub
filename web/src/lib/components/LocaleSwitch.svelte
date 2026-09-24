<script lang="ts">
	import Languages from '@lucide/svelte/icons/languages';
	import { getLocale, LOCALE_NAMES, locales, setLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	const current = getLocale();
</script>

<!-- Switching stores the choice and reloads, so every text follows at once. -->
<label
	class="relative inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-2 py-1 text-sm text-ink-2 focus-within:outline-2 focus-within:outline-accent hover:text-ink"
>
	<Languages size={15} aria-hidden="true" />
	<span class="sr-only">{m.locale_label()}</span>
	<select
		class="cursor-pointer appearance-none bg-transparent pr-1 outline-none"
		value={current}
		onchange={(event) => setLocale(event.currentTarget.value as typeof current)}
	>
		{#each locales as locale (locale)}
			<option value={locale} lang={locale}>{LOCALE_NAMES[locale]}</option>
		{/each}
	</select>
</label>
