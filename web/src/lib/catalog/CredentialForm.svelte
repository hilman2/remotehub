<script lang="ts">
	import type { Credential, CredentialInput } from '$lib/api/catalog';
	import { untrack } from 'svelte';
	import { m } from '$lib/paraglide/messages';
	import FieldsEditor, { type EditedField } from '$lib/vault/FieldsEditor.svelte';
	import IconPicker from '$lib/vault/IconPicker.svelte';
	import SecretFields, { secretInput } from './SecretFields.svelte';

	let {
		collectionId,
		places,
		credential = null,
		onsubmit,
		oncancel
	}: {
		/** Where a new credential goes (#190). */
		collectionId: string;
		/** The collections it may go into, with their path. */
		places: { id: string; path: string }[];
		credential?: Credential | null;
		onsubmit: (input: CredentialInput) => void;
		oncancel: () => void;
	} = $props();

	const start = untrack(() => $state.snapshot(credential));
	let collection = $state(untrack(() => start?.collection_id ?? collectionId));
	let name = $state(start?.name ?? '');
	let username = $state(start?.username ?? '');
	let domain = $state(start?.domain ?? '');
	let password = $state('');
	let url = $state(start?.url ?? '');
	let notes = $state(start?.notes ?? '');
	let icon = $state(start?.icon ?? 0);
	let fields = $state<EditedField[]>(
		(start?.fields ?? []).map((field) => ({
			name: field.name,
			value: field.value ?? '',
			protected: !!field.protected,
			stored: !!field.protected
		}))
	);

	function submit(event: SubmitEvent) {
		event.preventDefault();
		const input: CredentialInput = {
			collection_id: collection,
			name,
			username,
			domain,
			url,
			notes,
			icon,
			// A stored protected field left empty keeps its value.
			fields: fields.map((field) =>
				field.protected && field.stored && field.value === ''
					? { name: field.name, protected: true }
					: { name: field.name, protected: field.protected, value: field.value }
			)
		};
		// Updating without a new password keeps the stored one.
		Object.assign(
			input,
			secretInput('password', !!start, {
				password,
				privateKey: '',
				passphrase: '',
				certificate: ''
			})
		);
		password = '';
		onsubmit(input);
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit} autocomplete="off">
	<label class="block text-sm font-medium" for="credential-name">{m.field_name()}</label>
	<input id="credential-name" class={field} required maxlength="200" bind:value={name} />

	<label class={label} for="credential-collection">{m.vault_collection()}</label>
	<select id="credential-collection" class={field} required bind:value={collection}>
		{#each places as place (place.id)}
			<option value={place.id}>{place.path}</option>
		{/each}
	</select>

	<div class="grid grid-cols-2 gap-3">
		<div>
			<label class={label} for="credential-username">{m.field_username()}</label>
			<input
				id="credential-username"
				class={field}
				maxlength="256"
				spellcheck="false"
				bind:value={username}
			/>
		</div>
		<div>
			<label class={label} for="credential-domain">{m.field_domain()}</label>
			<input
				id="credential-domain"
				class={field}
				maxlength="256"
				spellcheck="false"
				bind:value={domain}
			/>
		</div>
	</div>

	<SecretFields id="credential" kind="password" keep={!!start} bind:password />

	<label class={label} for="credential-url">{m.field_url()}</label>
	<input id="credential-url" class={field} maxlength="2000" spellcheck="false" bind:value={url} />
	<label class={label} for="credential-notes">{m.field_notes()}</label>
	<textarea id="credential-notes" class={field} rows="3" maxlength="10000" bind:value={notes}
	></textarea>
	<FieldsEditor id="credential" bind:fields />
	<label class={label} for="credential-icon">{m.vault_icon()}</label>
	<IconPicker id="credential-icon" bind:value={icon} />

	<div class="mt-5 flex justify-end gap-2">
		<button
			type="button"
			class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
			onclick={oncancel}
		>
			{m.action_cancel()}
		</button>
		<button
			type="submit"
			class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
		>
			{start ? m.action_save() : m.action_create()}
		</button>
	</div>
</form>
