<script lang="ts">
	/**
	 * Carries out an approved recovery (#95) in this browser: reads the
	 * organisation's private key, opens the vault, and then either gives the
	 * owner a one-time recovery key or moves the entries into a shared folder.
	 */
	import { allows, loadTree, type Tree } from '$lib/api/catalog';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import { pathTo } from '$lib/catalog/tree';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { importInto } from './shared-kdbx';
	import {
		attachmentsOf,
		completeHandover,
		completeWithOneTimeKey,
		openVault,
		readPrivateKey,
		type Recovery
	} from './recovery';
	import { asKdbx, readFile } from './vault';

	let { recovery, ondone }: { recovery: Recovery; ondone: () => void } = $props();

	let useText = $state(false);
	let file = $state<File | null>(null);
	let passphrase = $state('');
	let text = $state('');
	let folderId = $state('');
	let tree = $state<Tree | null>(null);
	let busy = $state(false);
	let error = $state<string | null>(null);
	/** The one-time recovery key for the owner, shown once. */
	let oneTime = $state<string | null>(null);
	let imported = $state<string | null>(null);

	/** The folders the entries may go into, by their path. */
	const folders = $derived.by(() => {
		const all = tree;
		if (!all) return [];
		return all.folders
			.filter((folder) => allows(folder.role, 'edit'))
			.map((folder) => ({
				id: folder.id,
				path: pathTo(all, folder.id)
					.map((f) => f.name)
					.join(' / ')
			}))
			.sort((a, b) => a.path.localeCompare(b.path, getLocale()));
	});

	$effect(() => {
		if (recovery.kind !== 'handover') return;
		loadTree().then((result) => {
			if (result.ok) tree = result.data;
			else error = errorMessage(result.code);
		});
	});

	async function run(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		const privateKey = await readPrivateKey(useText ? { text } : { file: file!, passphrase });
		if (!privateKey) {
			error = m.recovery_key_unreadable();
			return;
		}
		busy = true;
		try {
			const opened = await openVault(recovery.id, privateKey);
			if (!opened.ok) {
				error = problemMessage(opened);
				return;
			}
			if (!opened.data) {
				error = m.recovery_key_wrong();
				return;
			}
			const { key, entries } = opened.data;
			if (recovery.kind === 'passphrase') {
				const done = await completeWithOneTimeKey(recovery.id, key);
				if (done.ok) oneTime = done.data;
				else error = problemMessage(done);
				return;
			}
			if (!tree) return;
			const from = attachmentsOf(recovery.id);
			const moved = await asKdbx(entries, (ref) => readFile(key, ref, from));
			const result = await importInto(tree, folderId, moved);
			imported = m.kdbx_imported({ count: result.created });
			if (result.failed.length > 0) {
				error = m.kdbx_not_imported({ names: result.failed.join(', ') });
				return;
			}
			const done = await completeHandover(recovery.id);
			if (!done.ok) error = problemMessage(done);
		} finally {
			busy = false;
			ondone();
		}
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

{#if oneTime}
	<p class="text-sm">{m.recovery_one_time_hint({ name: recovery.user_name })}</p>
	<p
		class="mt-4 rounded-lg bg-surface-2 p-3 font-mono text-lg break-all"
		data-testid="one-time-key"
	>
		{oneTime}
	</p>
{:else}
	<form onsubmit={run}>
		{#if useText}
			<label class="block text-sm font-medium" for="recovery-text">{m.recovery_key_text()}</label>
			<textarea
				id="recovery-text"
				class="{field} font-mono"
				rows="3"
				required
				spellcheck="false"
				autocomplete="off"
				bind:value={text}></textarea>
		{:else}
			<label class="block text-sm font-medium" for="recovery-file">{m.recovery_key_file()}</label>
			<input
				id="recovery-file"
				class={field}
				type="file"
				accept=".json,application/json"
				required
				onchange={(event) => (file = event.currentTarget.files?.[0] ?? null)}
			/>
			<label class={label} for="recovery-file-passphrase">{m.recovery_key_passphrase()}</label>
			<input
				id="recovery-file-passphrase"
				class={field}
				type="password"
				required
				autocomplete="off"
				bind:value={passphrase}
			/>
		{/if}
		<button
			type="button"
			class="mt-2 text-xs text-ink-3 underline"
			onclick={() => (useText = !useText)}
		>
			{useText ? m.recovery_use_file() : m.recovery_use_text()}
		</button>
		{#if recovery.kind === 'handover'}
			<label class={label} for="recovery-folder">{m.recovery_target_folder()}</label>
			<select id="recovery-folder" class={field} required bind:value={folderId}>
				<option value="" disabled></option>
				{#each folders as folder (folder.id)}
					<option value={folder.id}>{folder.path}</option>
				{/each}
			</select>
		{/if}
		<button
			type="submit"
			class="mt-5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
			disabled={busy}
		>
			{m.recovery_carry_out()}
		</button>
	</form>
{/if}
{#if imported}
	<p class="mt-3 text-sm" role="status">{imported}</p>
{/if}
{#if error}
	<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
{/if}
