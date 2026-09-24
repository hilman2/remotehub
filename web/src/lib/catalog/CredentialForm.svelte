<script lang="ts">
	import type { Credential, CredentialInput, CredentialKind } from '$lib/api/catalog';
	import { untrack } from 'svelte';
	import { m } from '$lib/paraglide/messages';
	import { CREDENTIAL_KIND_LABELS } from './labels';

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

	/** Keys and certificates are a few KiB; anything much larger is the wrong file. */
	const MAX_FILE_BYTES = 64 * 1024;

	const start = untrack(() => $state.snapshot(credential));
	let name = $state(start?.name ?? '');
	let kind = $state<CredentialKind>(start?.kind ?? 'password');
	let username = $state(start?.username ?? '');
	let domain = $state(start?.domain ?? '');
	let password = $state('');
	let privateKey = $state('');
	let passphrase = $state('');
	let certificate = $state('');

	async function load(event: Event, into: (text: string) => void) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (file && file.size <= MAX_FILE_BYTES) into(await file.text());
	}

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
		if (kind === 'password') {
			if (password || !start) input.password = password;
			password = '';
		} else if (privateKey || !start) {
			input.private_key = privateKey;
			if (passphrase) input.passphrase = passphrase;
			if (certificate.trim()) input.certificate = certificate;
			// The key stays for a retry with another passphrase.
			passphrase = '';
		}
		onsubmit(input);
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
	const fileButton =
		'mt-3 cursor-pointer rounded-md px-2 py-0.5 text-xs text-ink-3 underline hover:text-ink focus-within:outline-2';
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

	{#if kind === 'password'}
		<label class={label} for="credential-password">{m.field_password()}</label>
		<input
			id="credential-password"
			class={field}
			type="password"
			autocomplete="new-password"
			required={!start}
			bind:value={password}
			aria-describedby={start ? 'credential-secret-hint' : undefined}
		/>
		{#if start}
			<p id="credential-secret-hint" class="mt-1 text-xs text-ink-3">
				{m.field_password_keep()}
			</p>
		{/if}
	{:else}
		<div class="flex items-baseline justify-between">
			<label class={label} for="credential-private-key">{m.field_private_key()}</label>
			<label class={fileButton}>
				{m.field_load_private_key()}
				<input
					type="file"
					class="sr-only"
					onchange={(event) => load(event, (text) => (privateKey = text))}
				/>
			</label>
		</div>
		<textarea
			id="credential-private-key"
			class="{field} h-28 font-mono text-xs"
			spellcheck="false"
			required={!start}
			bind:value={privateKey}
			aria-describedby={start ? 'credential-secret-hint' : undefined}></textarea>
		{#if start}
			<p id="credential-secret-hint" class="mt-1 text-xs text-ink-3">{m.field_key_keep()}</p>
		{/if}

		<label class={label} for="credential-passphrase">{m.field_passphrase()}</label>
		<input
			id="credential-passphrase"
			class={field}
			type="password"
			autocomplete="off"
			bind:value={passphrase}
		/>

		<div class="flex items-baseline justify-between">
			<label class={label} for="credential-certificate">{m.field_certificate()}</label>
			<label class={fileButton}>
				{m.field_load_certificate()}
				<input
					type="file"
					class="sr-only"
					onchange={(event) => load(event, (text) => (certificate = text))}
				/>
			</label>
		</div>
		<textarea
			id="credential-certificate"
			class="{field} h-16 font-mono text-xs"
			spellcheck="false"
			bind:value={certificate}></textarea>
	{/if}

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
