<script lang="ts">
	import type { Credential, CredentialInput, CredentialKind } from '$lib/api/catalog';
	import { untrack } from 'svelte';
	import { m } from '$lib/paraglide/messages';
	import { CREDENTIAL_KIND_LABELS } from './labels';
	import SecretFields, { secretInput } from './SecretFields.svelte';

	let {
		folderId,
		credential = null,
		onsubmit,
		oncancel
	}: {
		folderId: string;
		credential?: Credential | null;
		onsubmit: (input: CredentialInput) => void;
		oncancel: () => void;
	} = $props();

	const start = untrack(() => $state.snapshot(credential));
	let name = $state(start?.name ?? '');
	let kind = $state<CredentialKind>(start?.kind ?? 'password');
	let username = $state(start?.username ?? '');
	let domain = $state(start?.domain ?? '');
	let password = $state('');
	let privateKey = $state('');
	let passphrase = $state('');
	let certificate = $state('');

	function submit(event: SubmitEvent) {
		event.preventDefault();
		const input: CredentialInput = {
			folder_id: start?.folder_id ?? folderId,
			name,
			username,
			domain
		};
		if (!start) input.kind = kind;
		// Updating without new secrets keeps the stored ones.
		Object.assign(
			input,
			secretInput(kind, !!start, { password, privateKey, passphrase, certificate })
		);
		// The key stays for a retry with another passphrase.
		password = passphrase = '';
		onsubmit(input);
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit} autocomplete="off">
	<label class="block text-sm font-medium" for="credential-name">{m.field_name()}</label>
	<input id="credential-name" class={field} required maxlength="200" bind:value={name} />

	<label class={label} for="credential-kind">{m.credential_kind()}</label>
	<select id="credential-kind" class={field} disabled={!!start} bind:value={kind}>
		{#each Object.entries(CREDENTIAL_KIND_LABELS) as [value, text] (value)}
			<option {value}>{text()}</option>
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

	<SecretFields
		id="credential"
		{kind}
		keep={!!start}
		bind:password
		bind:privateKey
		bind:passphrase
		bind:certificate
	/>

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
