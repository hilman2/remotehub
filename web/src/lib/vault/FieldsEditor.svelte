<script module lang="ts">
	/** A custom field while it is edited (#98). */
	export interface EditedField {
		name: string;
		value: string;
		protected: boolean;
		/**
		 * A protected field that has a value on the server already: left
		 * empty, it keeps that one.
		 */
		stored?: boolean;
	}
</script>

<script lang="ts">
	/** Custom fields as KeePass has them: a name, a value, protected or not. */
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { m } from '$lib/paraglide/messages';

	let { id, fields = $bindable([]) }: { id: string; fields?: EditedField[] } = $props();

	const input = 'w-full rounded-lg border border-line bg-page px-2 py-1.5 text-sm';
</script>

<fieldset class="mt-3">
	<legend class="text-sm font-medium">{m.vault_fields()}</legend>
	{#each fields as field, index (index)}
		<div class="mt-2 grid grid-cols-[1fr_1.5fr_auto_auto] items-center gap-2">
			<input
				class={input}
				aria-label={m.vault_field_name()}
				maxlength="100"
				required
				bind:value={field.name}
			/>
			<input
				id="{id}-field-{index}"
				class="{input} {field.protected ? 'font-mono' : ''}"
				aria-label={m.vault_field_value()}
				type={field.protected ? 'password' : 'text'}
				autocomplete="off"
				placeholder={field.stored ? m.vault_field_keep() : ''}
				maxlength="10000"
				bind:value={field.value}
			/>
			<label class="flex items-center gap-1 text-xs text-ink-2">
				<input type="checkbox" bind:checked={field.protected} disabled={field.stored} />
				{m.vault_field_protected()}
			</label>
			<button
				type="button"
				class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
				title={m.vault_field_remove({ name: field.name })}
				onclick={() => (fields = fields.filter((_, i) => i !== index))}
			>
				<X size={14} aria-hidden="true" />
				<span class="sr-only">{m.vault_field_remove({ name: field.name })}</span>
			</button>
		</div>
	{/each}
	<button
		type="button"
		class="mt-2 inline-flex items-center gap-1 rounded-md px-2 py-1 text-sm text-ink-2 hover:bg-surface-2 hover:text-ink"
		onclick={() => (fields = [...fields, { name: '', value: '', protected: false }])}
	>
		<Plus size={14} aria-hidden="true" />
		{m.vault_field_add()}
	</button>
</fieldset>
