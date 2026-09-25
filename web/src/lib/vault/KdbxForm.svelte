<script lang="ts">
	/**
	 * The form for a KeePass file (#99): to import, the file and its
	 * password; to export, the password the new file gets, twice.
	 */
	import { m } from '$lib/paraglide/messages';
	import { readKdbx, type KdbxEntry } from './kdbx';

	let {
		mode,
		busy = false,
		onimport,
		onexport
	}: {
		mode: 'import' | 'export';
		busy?: boolean;
		onimport?: (entries: KdbxEntry[]) => void;
		onexport?: (password: string) => void;
	} = $props();

	/** As long as the vault's own passphrase must be. */
	const MIN_PASSWORD = 12;

	let file = $state<File | null>(null);
	let password = $state('');
	let again = $state('');
	let error = $state<string | null>(null);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		if (mode === 'export') {
			if (password.length < MIN_PASSWORD) {
				error = m.vault_passphrase_short({ count: MIN_PASSWORD });
				return;
			}
			if (password !== again) {
				error = m.vault_passphrase_mismatch();
				return;
			}
			onexport?.(password);
			return;
		}
		if (!file) return;
		try {
			onimport?.(await readKdbx(await file.arrayBuffer(), password));
		} catch {
			error = m.kdbx_unreadable();
		}
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit}>
	{#if mode === 'import'}
		<label class="block text-sm font-medium" for="kdbx-file">{m.kdbx_file()}</label>
		<input
			id="kdbx-file"
			class={field}
			type="file"
			accept=".kdbx"
			required
			onchange={(event) => (file = event.currentTarget.files?.[0] ?? null)}
		/>
		<label class={label} for="kdbx-password">{m.kdbx_password()}</label>
		<input
			id="kdbx-password"
			class={field}
			type="password"
			autocomplete="off"
			bind:value={password}
		/>
	{:else}
		<p class="text-sm text-ink-2">{m.kdbx_export_hint()}</p>
		<label class={label} for="kdbx-new-password">{m.kdbx_new_password()}</label>
		<input
			id="kdbx-new-password"
			class={field}
			type="password"
			autocomplete="new-password"
			required
			bind:value={password}
		/>
		<label class={label} for="kdbx-new-password-again">{m.field_passphrase_again()}</label>
		<input
			id="kdbx-new-password-again"
			class={field}
			type="password"
			autocomplete="new-password"
			required
			bind:value={again}
		/>
	{/if}
	{#if error}
		<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
	{/if}
	<button
		type="submit"
		class="mt-5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
		disabled={busy}
	>
		{mode === 'import' ? m.kdbx_import() : m.kdbx_export()}
	</button>
</form>
