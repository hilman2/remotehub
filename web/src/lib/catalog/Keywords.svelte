<script lang="ts">
	/**
	 * A device's search words (#215): everyone who sees the device may change
	 * them, and everyone's search finds it by them.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { setKeywords, type Device } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let { device, onsaved }: { device: Device; onsaved: () => void } = $props();

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	let text = $state('');
	let error = $state<string | null>(null);
	let saved = $state(false);

	$effect.pre(() => {
		// Another device, or the words as saved: start from them.
		text = device.keywords;
		error = null;
	});

	const changed = $derived(text.trim() !== device.keywords);

	async function save(event: SubmitEvent) {
		event.preventDefault();
		const result = await setKeywords(device.id, text);
		if (!result.ok) {
			error = errorMessage(result.code);
			saved = false;
			return;
		}
		error = null;
		saved = true;
		onsaved();
	}
</script>

<form class="flex flex-col gap-2" onsubmit={save}>
	<label class="sr-only" for="keywords-{device.id}">{m.device_keywords()}</label>
	<div class="flex items-center gap-2">
		<input
			id="keywords-{device.id}"
			type="text"
			class="h-10 min-w-0 flex-1 rounded-lg border border-line bg-page px-3 text-sm"
			maxlength="500"
			placeholder={m.device_keywords_placeholder()}
			bind:value={text}
			oninput={() => (saved = false)}
		/>
		<button
			type="submit"
			class="h-10 rounded-xl border border-line-strong bg-surface px-3.5 text-sm hover:bg-surface-2 disabled:opacity-50"
			disabled={!changed}
		>
			{m.action_save()}
		</button>
	</div>
	<p class="text-xs text-ink-3">{m.device_keywords_hint()}</p>
	{#if device.keywords_changed_by && device.keywords_changed_at}
		<p class="text-xs text-ink-3">
			{m.device_keywords_changed({
				name: device.keywords_changed_by,
				when: time.format(new Date(device.keywords_changed_at))
			})}
		</p>
	{/if}
</form>

{#if error}
	<p class="flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{:else if saved && !changed}
	<p class="flex items-center gap-2 text-sm" role="status">
		<CircleCheck size={16} class="text-ok" aria-hidden="true" />
		{m.device_keywords_saved()}
	</p>
{/if}
