<script lang="ts">
	/** Whether and in which language a sign-in code goes out by mail (#145). */
	import { LOCALE_NAMES, locales } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let {
		sendMail = $bindable(true),
		language = $bindable('en')
	}: { sendMail?: boolean; language?: string } = $props();
</script>

<div class="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2 text-sm">
	<label class="flex items-center gap-2">
		<input type="checkbox" bind:checked={sendMail} />
		{m.users_send_mail()}
	</label>
	{#if sendMail}
		<label class="flex items-center gap-2">
			{m.users_mail_language()}
			<select class="rounded-lg border border-line bg-page px-2 py-1" bind:value={language}>
				{#each locales as locale (locale)}
					<option value={locale} lang={locale}>{LOCALE_NAMES[locale]}</option>
				{/each}
			</select>
		</label>
	{/if}
</div>
