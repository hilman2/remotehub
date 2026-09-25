<script lang="ts">
	/**
	 * A password field with a generator (#100): a new password is shown, so
	 * the person sees what they are about to save.
	 */
	import Dices from '@lucide/svelte/icons/dices';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import { m } from '$lib/paraglide/messages';
	import { generatePassword } from './generate';

	let {
		id,
		value = $bindable(''),
		required = false,
		describedby
	}: { id: string; value?: string; required?: boolean; describedby?: string } = $props();

	let visible = $state(false);

	const button = 'rounded-md p-1.5 text-ink-3 hover:bg-surface-2 hover:text-ink';
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
	<button
		type="button"
		class={button}
		title={m.vault_generate()}
		onclick={() => {
			value = generatePassword();
			visible = true;
		}}
	>
		<Dices size={16} aria-hidden="true" />
		<span class="sr-only">{m.vault_generate()}</span>
	</button>
</div>
