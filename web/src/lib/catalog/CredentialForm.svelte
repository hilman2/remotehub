<script lang="ts">
	import type { Credential, CredentialInput } from '$lib/api/catalog';
	import { untrack } from 'svelte';
	import { m } from '$lib/paraglide/messages';

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
	let username = $state(start?.username ?? '');
	let domain = $state(start?.domain ?? '');
	let password = $state('');

	function submit(event: SubmitEvent) {
		event.preventDefault();
		const input: CredentialInput = {
			folder_id: start?.folder_id ?? folderId,
			name,
			username,
			domain
		};
		// Updating without a new password keeps the stored one.
		if (password || !start) input.password = password;
		password = '';
		onsubmit(input);
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit} autocomplete="off">
	<label class="block text-sm font-medium" for="credential-name">{m.field_name()}</label>
	<input id="credential-name" class={field} required maxlength="200" bind:value={name} />

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

	<label class={label} for="credential-password">{m.field_password()}</label>
	<input
		id="credential-password"
		class={field}
		type="password"
		autocomplete="new-password"
		required={!start}
		bind:value={password}
		aria-describedby={start ? 'credential-password-hint' : undefined}
	/>
	{#if start}
		<p id="credential-password-hint" class="mt-1 text-xs text-ink-3">{m.field_password_keep()}</p>
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
