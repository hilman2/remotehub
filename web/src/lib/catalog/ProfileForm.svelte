<script lang="ts">
	/**
	 * Creates or changes a login profile (#192): a login many devices share.
	 * It lies in a device folder, and devices in that folder and below may
	 * use it; the top level is for administrators.
	 */
	import { untrack } from 'svelte';
	import type { CredentialKind, Profile, ProfileInput } from '$lib/api/catalog';
	import { m } from '$lib/paraglide/messages';
	import { CREDENTIAL_KIND_LABELS } from './labels';
	import SecretFields, { secretInput } from './SecretFields.svelte';

	let {
		profile = null,
		folderId,
		places,
		topLevel,
		onsubmit,
		oncancel
	}: {
		/** The one to change; null: a new one. */
		profile?: Profile | null;
		/** Where a new one goes; null: the top level. */
		folderId: string | null;
		/** The folders it may go into, with their path. */
		places: { id: string; path: string }[];
		/** Whether the top level is offered. */
		topLevel: boolean;
		onsubmit: (input: ProfileInput) => void;
		oncancel: () => void;
	} = $props();

	const start = untrack(() => $state.snapshot(profile));
	let name = $state(start?.name ?? '');
	// '' stands for the top level: a select holds strings.
	let folder = $state(untrack(() => (start ? start.folder_id : folderId) ?? ''));
	let username = $state(start?.username ?? '');
	let domain = $state(start?.domain ?? '');
	let kind = $state<CredentialKind>(start?.secret_kind ?? 'password');
	let password = $state('');
	let privateKey = $state('');
	let passphrase = $state('');
	let certificate = $state('');

	// Another kind needs its secret anew; the same kind keeps it if left empty.
	const keep = $derived(!!start && start.secret_kind === kind);

	function submit(event: SubmitEvent) {
		event.preventDefault();
		onsubmit({
			folder_id: folder || null,
			name,
			username,
			domain,
			secret_kind: kind,
			...secretInput(kind, keep, { password, privateKey, passphrase, certificate })
		});
		// The key stays for a retry with another passphrase.
		password = passphrase = '';
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit} autocomplete="off">
	<label class="block text-sm font-medium" for="profile-name">{m.field_name()}</label>
	<input id="profile-name" class={field} required maxlength="200" bind:value={name} />

	<label class={label} for="profile-folder">{m.profile_folder()}</label>
	<select id="profile-folder" class={field} bind:value={folder}>
		{#if topLevel}
			<option value="">{m.profile_top_level()}</option>
		{/if}
		{#each places as place (place.id)}
			<option value={place.id}>{place.path}</option>
		{/each}
	</select>
	<p class="mt-1 text-xs text-ink-3">{m.profile_folder_hint()}</p>

	<div class="grid grid-cols-2 gap-3">
		<div>
			<label class={label} for="profile-username">{m.field_username()}</label>
			<input
				id="profile-username"
				class={field}
				maxlength="256"
				spellcheck="false"
				bind:value={username}
			/>
		</div>
		<div>
			<label class={label} for="profile-domain">{m.field_domain()}</label>
			<input
				id="profile-domain"
				class={field}
				maxlength="256"
				spellcheck="false"
				bind:value={domain}
			/>
		</div>
	</div>

	<label class={label} for="profile-kind">{m.credential_kind()}</label>
	<select id="profile-kind" class={field} bind:value={kind}>
		{#each Object.entries(CREDENTIAL_KIND_LABELS) as [value, text] (value)}
			<option {value}>{text()}</option>
		{/each}
	</select>

	<SecretFields
		id="profile"
		{kind}
		{keep}
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
