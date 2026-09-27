<script lang="ts">
	/**
	 * A password field with a generator (#100, #194): a new password is
	 * shown, so the person sees what they are about to save. The dice make
	 * one as the user's or the organisation's default says; the settings
	 * beside them change that for this once, or keep it as the user's own.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Dices from '@lucide/svelte/icons/dices';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import { onMount } from 'svelte';
	import { errorMessage } from '$lib/api/errors';
	import { removeOwnDefault, saveOwnDefault } from '$lib/api/generator';
	import { m } from '$lib/paraglide/messages';
	import GeneratorOptions from './GeneratorOptions.svelte';
	import { BUILT_IN, generate, type GeneratorSettings } from './generate';
	import { current, defaults, loadDefaults } from './generator.svelte';

	let {
		id,
		value = $bindable(''),
		required = false,
		describedby
	}: { id: string; value?: string; required?: boolean; describedby?: string } = $props();

	let visible = $state(false);
	let open = $state(false);
	let settings = $state<GeneratorSettings>({ ...BUILT_IN });
	/** Counts up for a new suggestion with the same settings. */
	let round = $state(0);
	const suggestion = $derived.by(() => {
		void round;
		return generate(settings);
	});
	let note = $state('');
	let failed = $state('');

	// Loaded ahead, so the dice answer at once.
	onMount(() => void loadDefaults());

	async function quick() {
		await loadDefaults();
		value = generate(current());
		visible = true;
	}

	async function toggle() {
		if (open) {
			open = false;
			return;
		}
		await loadDefaults();
		settings = { ...current() };
		note = '';
		failed = '';
		open = true;
	}

	function use() {
		value = suggestion;
		visible = true;
		open = false;
	}

	async function keepOwn() {
		const own = $state.snapshot(settings);
		const result = await saveOwnDefault(own);
		if (!result.ok) {
			failed = errorMessage(result.code);
			return;
		}
		if (defaults.stored) defaults.stored.own = own;
		failed = '';
		note = m.generator_own_kept();
	}

	async function useOrganisation() {
		const result = await removeOwnDefault();
		if (!result.ok) {
			failed = errorMessage(result.code);
			return;
		}
		if (defaults.stored) {
			defaults.stored.own = null;
			settings = { ...defaults.stored.organisation };
		}
		failed = '';
		note = '';
	}

	const button = 'rounded-md p-1.5 text-ink-3 hover:bg-surface-2 hover:text-ink';
	const action =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
</script>

<div class="mt-1 flex items-center gap-1">
	<input
		{id}
		class="w-full rounded-lg border border-line bg-page px-3 py-2 {visible ? 'font-mono' : ''}"
		type={visible ? 'text' : 'password'}
		autocomplete="new-password"
		spellcheck="false"
		{required}
		aria-describedby={describedby}
		bind:value
	/>
	<button
		type="button"
		class={button}
		title={visible ? m.vault_hide() : m.vault_show()}
		onclick={() => (visible = !visible)}
	>
		{#if visible}<EyeOff size={16} aria-hidden="true" />{:else}<Eye
				size={16}
				aria-hidden="true"
			/>{/if}
		<span class="sr-only">{visible ? m.vault_hide() : m.vault_show()}</span>
	</button>
	<button type="button" class={button} title={m.vault_generate()} onclick={quick}>
		<Dices size={16} aria-hidden="true" />
		<span class="sr-only">{m.vault_generate()}</span>
	</button>
	<button
		type="button"
		class={button}
		title={m.generator_settings()}
		aria-expanded={open}
		aria-controls="{id}-generator"
		onclick={toggle}
	>
		<SlidersHorizontal size={16} aria-hidden="true" />
		<span class="sr-only">{m.generator_settings()}</span>
	</button>
</div>

{#if open}
	<div
		id="{id}-generator"
		class="mt-2 rounded-lg border border-line bg-surface-2 p-3"
		role="group"
		aria-label={m.generator_settings()}
	>
		<GeneratorOptions bind:settings />
		<p class="mt-3 text-sm font-medium">{m.generator_example()}</p>
		<div class="mt-1 flex items-center gap-2">
			<!-- Not an <output>: a screen reader would read out every suggestion. -->
			<p
				class="min-h-8 min-w-0 flex-1 rounded-md bg-page px-2 py-1.5 font-mono text-sm break-all"
				data-testid="generator-suggestion"
			>
				{suggestion}
			</p>
			<button type="button" class={button} title={m.generator_again()} onclick={() => round++}>
				<RefreshCw size={16} aria-hidden="true" />
				<span class="sr-only">{m.generator_again()}</span>
			</button>
		</div>
		<div class="mt-3 flex flex-wrap items-center gap-2">
			<button type="button" class={primary} disabled={!suggestion} onclick={use}>
				{m.generator_use()}
			</button>
			<button type="button" class={action} disabled={!suggestion} onclick={keepOwn}>
				{m.generator_keep_own()}
			</button>
			{#if defaults.stored?.own}
				<button type="button" class={action} onclick={useOrganisation}>
					{m.generator_use_organisation()}
				</button>
			{/if}
		</div>
		{#if note || defaults.stored?.own}
			<p class="mt-2 flex items-center gap-2 text-xs" role="status">
				<CircleCheck size={14} class="text-ok" aria-hidden="true" />
				{note || m.generator_own_active()}
			</p>
		{/if}
		{#if failed}
			<p class="mt-2 flex items-center gap-2 text-xs text-critical" role="alert">
				<CircleAlert size={14} aria-hidden="true" />
				{failed}
			</p>
		{/if}
	</div>
{/if}
