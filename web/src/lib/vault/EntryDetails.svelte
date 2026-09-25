<script lang="ts">
	/**
	 * What an entry has besides its secret (#98): the address, notes and
	 * custom fields. Protected fields show only their name here.
	 */
	import Lock from '@lucide/svelte/icons/lock';
	import type { CredentialField } from '$lib/api/catalog';
	import { m } from '$lib/paraglide/messages';

	let { url, notes, fields }: { url: string; notes: string; fields: CredentialField[] } = $props();

	/** Only web addresses become links; anything else stays text. */
	const link = $derived(/^https?:\/\//i.test(url) ? url : null);
</script>

<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
	{#if url}
		<dt class="text-ink-2">{m.field_url()}</dt>
		<dd class="break-all">
			{#if link}
				<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- another site -->
				<a class="text-accent hover:underline" href={link} target="_blank" rel="noreferrer noopener"
					>{url}</a
				>
			{:else}
				{url}
			{/if}
		</dd>
	{/if}
	{#each fields as field (field.name)}
		<dt class="text-ink-2">{field.name}</dt>
		<dd class="break-all">
			{#if field.protected}
				<span class="inline-flex items-center gap-1.5 text-ink-2">
					<Lock size={13} aria-hidden="true" />
					{m.credential_hidden()}
				</span>
			{:else}
				{field.value}
			{/if}
		</dd>
	{/each}
	{#if notes}
		<dt class="text-ink-2">{m.field_notes()}</dt>
		<dd class="whitespace-pre-line">{notes}</dd>
	{/if}
</dl>
