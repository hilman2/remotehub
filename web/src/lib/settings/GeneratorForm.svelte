<script lang="ts">
	/**
	 * The organisation's default for the password generator (#194), for
	 * administrators. Users who keep a default of their own are not changed.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { onMount } from 'svelte';
	import { errorMessage } from '$lib/api/errors';
	import { loadGeneratorDefaults, saveOrganisationDefault } from '$lib/api/generator';
	import { m } from '$lib/paraglide/messages';
	import GeneratorOptions from '$lib/vault/GeneratorOptions.svelte';
	import { BUILT_IN, generate, type GeneratorSettings } from '$lib/vault/generate';
	import { defaults } from '$lib/vault/generator.svelte';

	let settings = $state<GeneratorSettings>({ ...BUILT_IN });
	let loaded = $state(false);
	let busy = $state(false);
	let saved = $state(false);
	let error = $state('');
	const suggestion = $derived(generate(settings));

	onMount(async () => {
		const result = await loadGeneratorDefaults();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		settings = result.data.organisation;
		loaded = true;
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		saved = false;
		const organisation = $state.snapshot(settings);
		const result = await saveOrganisationDefault(organisation);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		if (defaults.stored) defaults.stored.organisation = organisation;
		error = '';
		saved = true;
	}

	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
</script>

<!-- Only once loaded: what was stored would overwrite what someone chose meanwhile. -->
{#if loaded}
	<form class="mt-4" onsubmit={save}>
		<GeneratorOptions bind:settings />
		<p class="mt-3 text-sm font-medium">{m.generator_example()}</p>
		<!-- Not an <output>: a screen reader would read out every suggestion. -->
		<p
			class="mt-1 min-h-8 rounded-md bg-page px-2 py-1.5 font-mono text-sm break-all"
			data-testid="generator-suggestion"
		>
			{suggestion}
		</p>
		<div class="mt-5">
			<button type="submit" class={primary} disabled={busy || !suggestion}>
				{m.action_save()}
			</button>
		</div>
	</form>
{/if}

{#if error}
	<p class="mt-4 flex items-center gap-2 text-sm text-critical" role="alert">
		<CircleAlert size={16} aria-hidden="true" />
		{error}
	</p>
{/if}

{#if saved}
	<p class="mt-4 flex items-center gap-2 text-sm" role="status">
		<CircleCheck size={16} class="text-ok" aria-hidden="true" />
		{m.generator_saved()}
	</p>
{/if}
