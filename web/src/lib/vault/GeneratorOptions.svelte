<script lang="ts">
	/**
	 * The generator's settings (#194): at a password field for this once or
	 * as the user's default, on the settings page as the organisation's.
	 */
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { LENGTH, WORDS, strength, type GeneratorSettings } from './generate';

	let { settings = $bindable() }: { settings: GeneratorSettings } = $props();
	const uid = $props.id();

	const SETS = [
		['lower', m.generator_lower],
		['upper', m.generator_upper],
		['digits', m.generator_digits],
		['symbols', m.generator_symbols]
	] as const;
	const SEPARATORS: [string, () => string][] = [
		['-', m.generator_separator_hyphen],
		[' ', m.generator_separator_space],
		['.', m.generator_separator_dot],
		['_', m.generator_separator_underscore],
		['', m.generator_separator_none]
	];

	const noCharacters = $derived(
		!(settings.lower || settings.upper || settings.digits || settings.symbols)
	);

	/** A number typed in, held to the bounds the server keeps. */
	const within = (text: string, bounds: { min: number; max: number }) =>
		Math.min(bounds.max, Math.max(bounds.min, Math.round(Number(text)) || bounds.min));

	// These fields sit in other forms: Enter in them must not save those.
	const noSubmit = (event: KeyboardEvent) => {
		if (event.key === 'Enter') event.preventDefault();
	};

	const label = 'mt-3 block text-sm font-medium';
	const number = 'w-20 rounded-lg border border-line bg-page px-2 py-1.5 text-sm';
</script>

<fieldset>
	<legend class="text-sm font-medium">{m.generator_kind()}</legend>
	<div class="mt-1 flex flex-wrap gap-4 text-sm">
		<label class="flex items-center gap-2">
			<input type="radio" name="{uid}-kind" value="password" bind:group={settings.kind} />
			{m.generator_kind_password()}
		</label>
		<label class="flex items-center gap-2">
			<input type="radio" name="{uid}-kind" value="passphrase" bind:group={settings.kind} />
			{m.generator_kind_passphrase()}
		</label>
	</div>
</fieldset>

{#if settings.kind === 'password'}
	<label class={label} for="{uid}-length">{m.generator_length()}</label>
	<div class="mt-1 flex items-center gap-3">
		<input
			class="min-w-0 flex-1"
			type="range"
			min={LENGTH.min}
			max={LENGTH.max}
			aria-label={m.generator_length()}
			bind:value={settings.length}
		/>
		<input
			id="{uid}-length"
			class={number}
			type="number"
			min={LENGTH.min}
			max={LENGTH.max}
			value={settings.length}
			onchange={(event) => (settings.length = within(event.currentTarget.value, LENGTH))}
			onkeydown={noSubmit}
		/>
	</div>
	<fieldset class="mt-3">
		<legend class="text-sm font-medium">{m.generator_characters()}</legend>
		<div class="mt-1 grid gap-1 text-sm sm:grid-cols-2">
			{#each SETS as [set, text] (set)}
				<label class="flex items-center gap-2">
					<!-- bind:checked={settings[set]} does not write back from inside this each. -->
					<input
						type="checkbox"
						checked={settings[set]}
						onchange={(event) => (settings[set] = event.currentTarget.checked)}
					/>
					{text()}
				</label>
			{/each}
		</div>
		<label class="mt-2 flex items-center gap-2 text-sm">
			<input type="checkbox" bind:checked={settings.look_alikes} />
			{m.generator_look_alikes()}
		</label>
		{#if noCharacters}
			<p class="mt-1 text-xs text-critical" role="alert">{m.generator_no_characters()}</p>
		{/if}
	</fieldset>
{:else}
	<label class={label} for="{uid}-words">{m.generator_words()}</label>
	<div class="mt-1 flex items-center gap-3">
		<input
			class="min-w-0 flex-1"
			type="range"
			min={WORDS.min}
			max={WORDS.max}
			aria-label={m.generator_words()}
			bind:value={settings.words}
		/>
		<input
			id="{uid}-words"
			class={number}
			type="number"
			min={WORDS.min}
			max={WORDS.max}
			value={settings.words}
			onchange={(event) => (settings.words = within(event.currentTarget.value, WORDS))}
			onkeydown={noSubmit}
		/>
	</div>
	<label class={label} for="{uid}-separator">{m.generator_separator()}</label>
	<select
		id="{uid}-separator"
		class="mt-1 rounded-lg border border-line bg-page px-2 py-1.5 text-sm"
		bind:value={settings.separator}
	>
		{#each SEPARATORS as [separator, text] (separator)}
			<option value={separator}>{text()}</option>
		{/each}
	</select>
{/if}

<p class="mt-3 text-xs text-ink-3" data-testid="generator-strength">
	{m.generator_strength({ bits: new Intl.NumberFormat(formatLocale()).format(strength(settings)) })}
</p>
