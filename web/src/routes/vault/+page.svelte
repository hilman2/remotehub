<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Copy from '@lucide/svelte/icons/copy';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import Lock from '@lucide/svelte/icons/lock';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import { errorMessage } from '$lib/api/errors';
	import { formatLocale, getLocale } from '$lib/i18n';
	import { frequent, queryKey, rank, remember, type Pick } from '$lib/search/rank';
	import Dialog from '$lib/components/Dialog.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';
	import { unlocked } from '$lib/vault/unlocked.svelte';
	import {
		addPasskey,
		deleteEntry,
		loadVault,
		passkeysAvailable,
		readEntries,
		readPicks,
		removeUnlock,
		renewRecovery,
		resetVault,
		saveEntry,
		savePassphrase,
		savePicks,
		setUp,
		unlockWithPasskey,
		unlockWithPassphrase,
		unlockWithRecovery,
		type Entry,
		type EntryContent,
		type FileRef,
		type StoredVault,
		type UnlockKind,
		MAX_FILE,
		deleteFile,
		readFile,
		saveFile,
		withHistory,
		asKdbx,
		coverForOrganisation,
		oneTimeRecovery
	} from '$lib/vault/vault';
	import PasswordInput from '$lib/vault/PasswordInput.svelte';
	import KdbxForm from '$lib/vault/KdbxForm.svelte';
	import { writeKdbx, type KdbxEntry } from '$lib/vault/kdbx';
	import FileDown from '@lucide/svelte/icons/file-down';
	import FileUp from '@lucide/svelte/icons/file-up';
	import TotpCode from '$lib/vault/TotpCode.svelte';
	import { totpOf } from '$lib/vault/totp';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import FieldsEditor, { type EditedField } from '$lib/vault/FieldsEditor.svelte';
	import IconPicker from '$lib/vault/IconPicker.svelte';
	import SharedEntries from '$lib/vault/SharedEntries.svelte';
	import { FOLDER_ICON, icon } from '$lib/vault/icons';

	type Stage = 'loading' | 'setup' | 'recovery' | 'locked' | 'open';

	const MIN_PASSPHRASE = 12;
	const KIND_LABELS: Record<UnlockKind, () => string> = {
		passkey: m.vault_kind_passkey,
		passphrase: m.vault_kind_passphrase,
		recovery: m.vault_kind_recovery,
		organisation: m.vault_kind_organisation
	};
	const EMPTY: EntryContent = { title: '', username: '', password: '', url: '', notes: '' };

	let stage = $state<Stage>('loading');
	let vault = $state<StoredVault | null>(null);
	// The vault key: in memory only, kept in `unlocked` while the page is
	// loaded, so it is still open after a visit to another page.
	let key: CryptoKey | null = unlocked.key;
	let entries = $state<Entry[]>([]);
	let error = $state<string | null>(null);
	let busy = $state(false);

	let passphrase = $state('');
	let passphraseAgain = $state('');
	let recoveryText = $state('');
	let useRecovery = $state(false);
	let shownRecovery = $state('');
	let passkeyLabel = $state('');
	let revealed = $state<string | null>(null);
	let copied = $state<string | null>(null);

	let query = $state('');
	/** What the owner used after searching; sealed in the vault (#81). */
	let picks = $state<Pick[]>([]);

	/** The personal folder shown (#98); the top without one. */
	let folder = $state<string | null>(null);
	const folders = $derived(entries.filter((entry) => entry.content?.kind === 'folder'));
	/** The folders from the top down to the one shown. */
	const trail = $derived.by(() => {
		const way: Entry[] = [];
		let at = folders.find((f) => f.id === folder);
		while (at && !way.includes(at)) {
			way.unshift(at);
			at = folders.find((f) => f.id === at?.content?.parent);
		}
		return way;
	});
	const subfolders = $derived(folders.filter((f) => (f.content?.parent ?? null) === folder));
	/** Folders with their path, for choosing where an entry goes. */
	const places = $derived(
		folders
			.map((f) => {
				const names: string[] = [];
				let at: Entry | undefined = f;
				while (at && names.length < 20) {
					names.unshift(at.content?.title ?? '');
					at = folders.find((p) => p.id === at?.content?.parent);
				}
				return { id: f.id, path: names.join(' / ') };
			})
			.sort((a, b) => a.path.localeCompare(b.path, getLocale()))
	);

	/**
	 * Readable entries, best first; entries that do not open, last. Without
	 * a search, those of the folder shown; a search looks through all.
	 */
	const shown = $derived.by(() => {
		const readable = entries.filter((entry) => entry.content && entry.content.kind !== 'folder');
		if (queryKey(query)) {
			return rank(
				readable.map((entry) => ({
					key: entry.id,
					name: entry.content?.title ?? '',
					fields: [
						{ text: entry.content?.title ?? '', weight: 1 },
						{ text: entry.content?.username ?? '', weight: 0.8 },
						{ text: entry.content?.url ?? '', weight: 0.7 },
						{ text: entry.content?.notes ?? '', weight: 0.5 }
					],
					item: entry
				})),
				query,
				picks,
				Date.now(),
				getLocale()
			);
		}
		// Without a query, the ones used most come first.
		const order = frequent(picks, Date.now(), entries.length);
		const place = (entry: Entry) => {
			const at = order.indexOf(entry.id);
			return at < 0 ? order.length : at;
		};
		const here = readable.filter((entry) => (entry.content?.parent ?? null) === folder);
		return [
			...here.sort((a, b) => place(a) - place(b)),
			...(folder === null ? entries.filter((entry) => !entry.content) : [])
		];
	});

	let folderName = $state('');
	let folderOpen = $state(false);

	async function addFolder(event: SubmitEvent) {
		event.preventDefault();
		if (!key) return;
		const result = await saveEntry(key, null, {
			...EMPTY,
			kind: 'folder',
			parent: folder,
			title: folderName.trim(),
			icon: FOLDER_ICON
		});
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		folderName = '';
		folderOpen = false;
		await load();
	}

	/** Only an empty folder goes. */
	const empty = (id: string) => !entries.some((entry) => entry.content?.parent === id);

	async function removeFolder(id: string) {
		const parent = folders.find((f) => f.id === id)?.content?.parent ?? null;
		await remove(id);
		folder = parent;
	}

	/** The custom fields of the entry being edited. */
	let editedFields = $state<EditedField[]>([]);

	let editing = $state<{ id: string | null; content: EntryContent } | null>(null);
	let editorOpen = $state(false);
	let resetOpen = $state(false);
	let passphraseOpen = $state(false);

	async function load() {
		const result = await loadVault();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		vault = result.data;
		if (key && (await coverForOrganisation(key, vault))) {
			const again = await loadVault();
			if (again.ok) vault = again.data;
		}
		if (key) {
			[entries, picks] = await Promise.all([readEntries(key, vault), readPicks(key, vault)]);
			if (stage === 'loading') stage = 'open';
		} else stage = vault.unlocks.length === 0 ? 'setup' : 'locked';
	}

	$effect(() => {
		load();
	});

	function passphraseProblem(): string | null {
		if (passphrase.length < MIN_PASSPHRASE)
			return m.vault_passphrase_short({ count: MIN_PASSPHRASE });
		if (passphrase !== passphraseAgain) return m.vault_passphrase_mismatch();
		return null;
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		error = passphraseProblem();
		if (error) return;
		busy = true;
		const result = await setUp(passphrase);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		key = unlocked.key = result.key;
		shownRecovery = result.recovery;
		passphrase = passphraseAgain = '';
		stage = 'recovery';
	}

	async function opened(opening: CryptoKey | null) {
		if (!opening || !vault) {
			error = m.vault_wrong();
			return;
		}
		key = unlocked.key = opening;
		error = null;
		passphrase = recoveryText = '';
		// A one-time key from a recovery (#95) is replaced at once, and the
		// forgotten passphrase with it.
		if (useRecovery && oneTimeRecovery(vault)) {
			mustRenew = true;
			useRecovery = false;
			await newRecovery();
			return;
		}
		// Reads the entries, and wraps the key for the organisation if needed.
		await load();
		stage = 'open';
	}

	/** After a one-time recovery key: the new passphrase comes next. */
	let mustRenew = $state(false);

	/** Remembers that `entry` was used after searching for the current query. */
	function used(entry: Entry) {
		if (!key) return;
		picks = remember(picks, entry.id, query, Date.now());
		// A preference: if it is not stored, the vault still works.
		savePicks(key, picks);
	}

	async function unlock(event: SubmitEvent) {
		event.preventDefault();
		if (!vault) return;
		busy = true;
		const opening = useRecovery
			? await unlockWithRecovery(vault, recoveryText)
			: await unlockWithPassphrase(vault, passphrase);
		busy = false;
		await opened(opening);
	}

	async function unlockPasskey() {
		if (!vault) return;
		try {
			await opened(await unlockWithPasskey(vault));
		} catch {
			error = m.vault_wrong();
		}
	}

	function lock() {
		key = unlocked.key = null;
		entries = [];
		picks = [];
		query = '';
		revealed = null;
		stage = 'locked';
	}

	async function afterRecovery() {
		await load();
		if (key && vault) entries = await readEntries(key, vault);
		stage = 'open';
		if (mustRenew) {
			mustRenew = false;
			passphraseOpen = true;
		}
	}

	function edit(entry: Entry | null) {
		if (entry) used(entry);
		// Entries from before #98 have neither folder nor icon.
		const content = { ...(entry?.content ?? { ...EMPTY, parent: folder }) };
		content.parent ??= null;
		content.icon ??= 0;
		editing = { id: entry?.id ?? null, content };
		editedFields = (entry?.content?.fields ?? []).map((field) => ({ ...field }));
		pendingFiles = [];
		removedFiles = [];
		error = null;
		editorOpen = true;
	}

	/** Files chosen in the editor, sealed and stored on save. */
	let pendingFiles = $state<File[]>([]);
	/** Files removed in the editor, deleted once the entry is saved. */
	let removedFiles = $state<string[]>([]);

	function addFiles(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const chosen = [...(input.files ?? [])];
		input.value = '';
		if (chosen.some((file) => file.size > MAX_FILE)) {
			error = m.vault_file_too_large({ megabytes: MAX_FILE / 1024 / 1024 });
			return;
		}
		pendingFiles = [...pendingFiles, ...chosen];
	}

	let kdbxMode = $state<'import' | 'export' | null>(null);
	let kdbxOpen = $state(false);
	let kdbxResult = $state<string | null>(null);

	function openKdbx(mode: 'import' | 'export') {
		kdbxMode = mode;
		kdbxResult = null;
		error = null;
		kdbxOpen = true;
	}

	/** Takes a KeePass file's entries into the folder shown (#99). */
	async function importPersonal(imported: KdbxEntry[]) {
		if (!key) return;
		busy = true;
		const made: Record<string, string | null> = { '': folder };
		const folderOf = async (path: string[]): Promise<string | null> => {
			const at = path.join('\n');
			if (at in made) return made[at];
			const parent = await folderOf(path.slice(0, -1));
			const saved = await saveEntry(key!, null, {
				...EMPTY,
				kind: 'folder',
				parent,
				title: path[path.length - 1],
				icon: FOLDER_ICON
			});
			const id = saved.ok ? saved.id : parent;
			made[at] = id;
			return id;
		};
		let count = 0;
		for (const item of imported) {
			const attachments: FileRef[] = [];
			for (const file of item.files) {
				const saved = await saveFile(key, new File([file.data], file.name));
				if (saved) attachments.push(saved);
			}
			const saved = await saveEntry(key, null, {
				title: item.title,
				username: item.username,
				password: item.password,
				url: item.url,
				notes: item.notes,
				icon: item.icon,
				fields: item.fields,
				attachments,
				parent: await folderOf(item.path)
			});
			if (saved.ok) count += 1;
		}
		busy = false;
		kdbxResult = m.kdbx_imported({ count });
		await load();
	}

	/** Writes the whole vault into a new KeePass file (#99). */
	async function exportPersonal(password: string) {
		const opened = key;
		if (!opened) return;
		busy = true;
		const out = await asKdbx(entries, (ref) => readFile(opened, ref));
		const file = await writeKdbx(out, password, m.vault_title());
		busy = false;
		const url = URL.createObjectURL(new Blob([file], { type: 'application/octet-stream' }));
		const link = document.createElement('a');
		link.href = url;
		link.download = 'remotehub-vault.kdbx';
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
		kdbxResult = m.kdbx_exported({ count: out.length });
	}

	/** Opens a file in the browser and hands it over as a download. */
	async function download(ref: FileRef) {
		if (!key) return;
		const blob = await readFile(key, ref);
		if (!blob) {
			error = errorMessage('not_found');
			return;
		}
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.href = url;
		link.download = ref.name;
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
	}

	const when = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
	const kilobytes = new Intl.NumberFormat(formatLocale(), { style: 'unit', unit: 'kilobyte' });
	const size = (bytes: number) => kilobytes.format(Math.max(1, Math.round(bytes / 1024)));

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (!key || !editing) return;
		busy = true;
		const added: FileRef[] = [];
		for (const file of pendingFiles) {
			const saved = await saveFile(key, file);
			if (!saved) {
				busy = false;
				error = errorMessage('internal');
				return;
			}
			added.push(saved);
		}
		let content: EntryContent = {
			...editing.content,
			fields: editedFields.map(({ name, value, protected: hidden }) => ({
				name: name.trim(),
				value,
				protected: hidden
			})),
			attachments: [
				...(editing.content.attachments ?? []).filter((f) => !removedFiles.includes(f.id)),
				...added
			]
		};
		const before = entries.find((entry) => entry.id === editing?.id)?.content;
		if (before) content = withHistory(before, content);
		const result = await saveEntry(key, editing.id, content);
		busy = false;
		if (result.ok) {
			for (const id of removedFiles) await deleteFile(id);
		}
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		editorOpen = false;
		await load();
	}

	async function remove(id: string) {
		const result = await deleteEntry(id);
		if (!result.ok) error = errorMessage(result.code);
		editorOpen = false;
		await load();
	}

	async function copy(entry: Entry) {
		if (!entry.content) return;
		used(entry);
		await navigator.clipboard.writeText(entry.content.password);
		copied = entry.id;
		setTimeout(() => (copied = null), 2000);
	}

	async function newPasskey() {
		if (!key || !session.user) return;
		error = null;
		try {
			const result = await addPasskey(key, session.user.username, passkeyLabel.trim());
			if (!result.ok) {
				error =
					result.code === 'prf_unsupported' ? m.vault_prf_unsupported() : errorMessage(result.code);
				return;
			}
		} catch {
			error = m.vault_prf_unsupported();
			return;
		}
		passkeyLabel = '';
		await load();
	}

	async function changePassphrase(event: SubmitEvent) {
		event.preventDefault();
		error = passphraseProblem();
		if (error || !key) return;
		busy = true;
		const result = await savePassphrase(key, passphrase);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		passphrase = passphraseAgain = '';
		passphraseOpen = false;
		await load();
	}

	async function newRecovery() {
		if (!key) return;
		const result = await renewRecovery(key);
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		shownRecovery = result.recovery;
		stage = 'recovery';
	}

	async function dropUnlock(id: string) {
		const result = await removeUnlock(id);
		error = result.ok ? null : errorMessage(result.code);
		await load();
	}

	async function startOver() {
		const result = await resetVault();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		resetOpen = false;
		key = unlocked.key = null;
		entries = [];
		stage = 'setup';
		await load();
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
</script>

{#snippet passphraseFields()}
	<label class={label} for="vault-passphrase">{m.field_passphrase()}</label>
	<input
		id="vault-passphrase"
		type="password"
		class={field}
		required
		autocomplete="new-password"
		bind:value={passphrase}
	/>
	<label class={label} for="vault-passphrase-again">{m.field_passphrase_again()}</label>
	<input
		id="vault-passphrase-again"
		type="password"
		class={field}
		required
		autocomplete="new-password"
		bind:value={passphraseAgain}
	/>
{/snippet}

{#snippet problem()}
	{#if error}
		<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
{/snippet}

<div class="flex flex-wrap items-center gap-3">
	<h1 class="text-4xl font-semibold">{m.vault_title()}</h1>
	{#if stage === 'open'}
		<button type="button" class="{button} ml-auto" onclick={lock}>
			<Lock size={16} aria-hidden="true" />
			{m.vault_lock()}
		</button>
	{/if}
</div>
<p class="mt-3 flex items-center gap-2 text-sm text-ink-2">
	<Lock size={14} aria-hidden="true" />
	{m.vault_intro()}
</p>

{#if stage === 'setup'}
	<form class="mt-6 max-w-md rounded-card border border-line bg-surface p-6" onsubmit={create}>
		<h2 class="text-lg font-semibold">{m.vault_setup_title()}</h2>
		<p class="mt-1 text-sm text-ink-2">{m.vault_setup_hint()}</p>
		{@render passphraseFields()}
		<button type="submit" class="{primary} mt-5" disabled={busy}>{m.vault_set_up()}</button>
		{@render problem()}
	</form>
{:else if stage === 'recovery'}
	<section class="mt-6 max-w-md rounded-card border border-line bg-surface p-6">
		<h2 class="text-lg font-semibold">{m.vault_recovery_title()}</h2>
		{#if mustRenew}
			<p class="mt-1 text-sm" role="status">{m.vault_recovered_hint()}</p>
		{/if}
		<p class="mt-1 text-sm text-ink-2">{m.vault_recovery_hint()}</p>
		<p
			class="mt-4 rounded-lg bg-surface-2 p-3 font-mono text-lg break-all"
			data-testid="recovery-key"
		>
			{shownRecovery}
		</p>
		<button type="button" class="{primary} mt-5" onclick={afterRecovery}>
			{m.vault_recovery_saved()}
		</button>
	</section>
{:else if stage === 'locked'}
	<form class="mt-6 max-w-md rounded-card border border-line bg-surface p-6" onsubmit={unlock}>
		<h2 class="text-lg font-semibold">{m.vault_unlock_title()}</h2>
		{#if useRecovery}
			<label class={label} for="vault-recovery">{m.field_recovery_key()}</label>
			<input
				id="vault-recovery"
				class="{field} font-mono"
				required
				autocomplete="off"
				spellcheck="false"
				bind:value={recoveryText}
			/>
		{:else}
			<label class={label} for="vault-unlock-passphrase">{m.field_passphrase()}</label>
			<input
				id="vault-unlock-passphrase"
				type="password"
				class={field}
				required
				autocomplete="current-password"
				bind:value={passphrase}
			/>
		{/if}
		<div class="mt-5 flex flex-wrap gap-2">
			<button type="submit" class={primary} disabled={busy}>{m.vault_unlock()}</button>
			{#if passkeysAvailable() && vault?.unlocks.some((u) => u.kind === 'passkey')}
				<button type="button" class={button} onclick={unlockPasskey}>
					<Fingerprint size={16} aria-hidden="true" />
					{m.vault_unlock_passkey()}
				</button>
			{/if}
			<button type="button" class={button} onclick={() => (useRecovery = !useRecovery)}>
				{useRecovery ? m.vault_use_passphrase() : m.vault_use_recovery()}
			</button>
		</div>
		<button
			type="button"
			class="mt-4 text-xs text-ink-3 underline"
			onclick={() => (resetOpen = true)}
		>
			{m.vault_reset()}
		</button>
		{@render problem()}
	</form>
{:else if stage === 'open'}
	<div class="mt-6 flex items-center gap-3">
		<h2 class="text-lg font-semibold">{m.vault_entries()}</h2>
		<button type="button" class="{button} ml-auto" onclick={() => openKdbx('import')}>
			<FileUp size={16} aria-hidden="true" />
			{m.kdbx_import()}
		</button>
		<button type="button" class={button} onclick={() => openKdbx('export')}>
			<FileDown size={16} aria-hidden="true" />
			{m.kdbx_export()}
		</button>
		<button type="button" class={button} onclick={() => (folderOpen = true)}>
			<FolderPlus size={16} aria-hidden="true" />
			{m.vault_new_folder()}
		</button>
		<button type="button" class={button} onclick={() => edit(null)}>
			<Plus size={16} aria-hidden="true" />
			{m.vault_new_entry()}
		</button>
	</div>
	{#if !editorOpen}{@render problem()}{/if}
	<nav class="mt-3 flex flex-wrap items-center gap-1 text-sm" aria-label={m.vault_folders()}>
		<button
			type="button"
			class="rounded-md px-2 py-1 hover:bg-surface-2"
			aria-current={folder === null ? 'location' : undefined}
			onclick={() => (folder = null)}
		>
			{m.vault_title()}
		</button>
		{#each trail as step (step.id)}
			<span class="text-ink-3" aria-hidden="true">/</span>
			<button
				type="button"
				class="rounded-md px-2 py-1 hover:bg-surface-2"
				aria-current={folder === step.id ? 'location' : undefined}
				onclick={() => (folder = step.id)}
			>
				{step.content?.title}
			</button>
		{/each}
		{#if folder && empty(folder)}
			<button
				type="button"
				class="ml-2 rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
				onclick={() => folder && removeFolder(folder)}
			>
				{m.vault_remove_folder()}
			</button>
		{/if}
	</nav>
	{#if subfolders.length > 0 && !queryKey(query)}
		<ul class="mt-2 flex flex-wrap gap-2">
			{#each subfolders as sub (sub.id)}
				<li>
					<button type="button" class={button} onclick={() => (folder = sub.id)}>
						<FolderIcon size={16} class="text-ink-3" aria-hidden="true" />
						{sub.content?.title}
					</button>
				</li>
			{/each}
		</ul>
	{/if}
	{#if entries.length === 0}
		<p class="mt-3 text-sm text-ink-3">{m.vault_empty()}</p>
	{:else}
		<label class="relative mt-4 block max-w-md">
			<span class="sr-only">{m.vault_search()}</span>
			<Search size={16} class="absolute top-3 left-3 text-ink-3" aria-hidden="true" />
			<input
				type="search"
				class="h-10 w-full rounded-xl border border-line-strong bg-surface pr-3 pl-9 text-sm"
				placeholder={m.vault_search()}
				bind:value={query}
			/>
		</label>
		{#if shown.length === 0}
			<p class="mt-3 text-sm text-ink-2">{m.catalog_no_match()}</p>
		{/if}
		<ul class="mt-3 divide-y divide-line rounded-card border border-line bg-surface empty:hidden">
			{#each shown as entry (entry.id)}
				<li class="flex flex-wrap items-center gap-3 px-4 py-3 text-sm">
					{#if entry.content}
						{@const Icon = icon(entry.content.icon)}
						<Icon size={16} class="shrink-0 text-warning" aria-hidden="true" />
						<div class="min-w-0">
							<p class="font-medium">{entry.content.title}</p>
							<p class="truncate text-ink-2">
								{entry.content.username}{entry.content.url ? ` · ${entry.content.url}` : ''}
							</p>
							{#if revealed === entry.id}
								<p class="mt-1 font-mono break-all">{entry.content.password}</p>
							{/if}
							{#each entry.content.fields ?? [] as field (field.name)}
								{@const totp = totpOf(field.name, field.value)}
								<p class="mt-1 text-xs text-ink-2">
									{field.name}:
									{#if totp && revealed === entry.id}
										<TotpCode params={totp} />
									{:else if (field.protected || totp) && revealed !== entry.id}
										<span aria-hidden="true">••••••</span>
									{:else}
										<span class="font-mono break-all">{field.value}</span>
									{/if}
								</p>
							{/each}
							{#each entry.content.attachments ?? [] as file (file.id)}
								<button
									type="button"
									class="mt-1 block text-xs text-accent hover:underline"
									onclick={() => download(file)}
								>
									{file.name} ({size(file.size)})
								</button>
							{/each}
							{#if revealed === entry.id && entry.content.history?.length}
								<details class="mt-2 text-xs text-ink-2">
									<summary class="cursor-pointer">{m.vault_history()}</summary>
									<ul class="mt-1 space-y-1">
										{#each entry.content.history as earlier (earlier.at)}
											<li>
												<span class="tabular-nums">{when.format(new Date(earlier.at))}</span>:
												<span class="font-mono break-all" data-testid="earlier-password"
													>{earlier.password}</span
												>
											</li>
										{/each}
									</ul>
								</details>
							{/if}
						</div>
						<div class="ml-auto flex flex-wrap gap-2">
							<button
								type="button"
								class={button}
								onclick={() => {
									if (revealed !== entry.id) used(entry);
									revealed = revealed === entry.id ? null : entry.id;
								}}
							>
								{#if revealed === entry.id}
									<EyeOff size={16} aria-hidden="true" />
									{m.vault_hide()}
								{:else}
									<Eye size={16} aria-hidden="true" />
									{m.vault_show()}
								{/if}
							</button>
							<button type="button" class={button} onclick={() => copy(entry)}>
								<Copy size={16} aria-hidden="true" />
								{copied === entry.id ? m.vault_copied() : m.vault_copy()}
							</button>
							<button type="button" class={button} onclick={() => edit(entry)}>
								{m.catalog_edit()}
							</button>
						</div>
					{:else}
						<p class="flex items-center gap-2 text-ink-2">
							<CircleAlert size={16} class="text-critical" aria-hidden="true" />
							{m.vault_unreadable()}
						</p>
						<button type="button" class="{button} ml-auto" onclick={() => remove(entry.id)}>
							{m.catalog_delete()}
						</button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}

	<h2 class="mt-8 text-lg font-semibold">{m.vault_unlocks_title()}</h2>
	<ul class="mt-3 divide-y divide-line rounded-card border border-line bg-surface">
		{#each vault?.unlocks ?? [] as unlock (unlock.id)}
			<li class="flex items-center gap-3 px-4 py-2 text-sm">
				<span>{KIND_LABELS[unlock.kind]()}{unlock.label ? ` · ${unlock.label}` : ''}</span>
				{#if unlock.kind === 'passkey'}
					<button
						type="button"
						class="ml-auto rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
						onclick={() => dropUnlock(unlock.id)}
					>
						{m.vault_remove()}
					</button>
				{/if}
			</li>
		{/each}
	</ul>
	<div class="mt-3 flex flex-wrap items-end gap-2">
		{#if passkeysAvailable()}
			<label class="text-sm">
				<span class="block font-medium">{m.vault_passkey_label()}</span>
				<input
					class="mt-1 rounded-lg border border-line bg-page px-3 py-1.5"
					bind:value={passkeyLabel}
				/>
			</label>
			<button type="button" class={button} onclick={newPasskey}>
				<Fingerprint size={16} aria-hidden="true" />
				{m.vault_add_passkey()}
			</button>
		{/if}
		<button type="button" class={button} onclick={() => (passphraseOpen = true)}>
			{m.vault_change_passphrase()}
		</button>
		<button type="button" class={button} onclick={newRecovery}>{m.vault_new_recovery()}</button>
	</div>
{/if}

{#if stage !== 'loading'}
	<SharedEntries {query} />
{/if}

<Dialog bind:open={editorOpen} title={editing?.id ? m.catalog_edit() : m.vault_new_entry()}>
	{#if editorOpen && editing}
		<form onsubmit={save}>
			<label class="block text-sm font-medium" for="entry-title">{m.field_title()}</label>
			<input
				id="entry-title"
				class={field}
				required
				maxlength="200"
				bind:value={editing.content.title}
			/>
			<label class={label} for="entry-username">{m.field_username()}</label>
			<input
				id="entry-username"
				class={field}
				autocomplete="off"
				bind:value={editing.content.username}
			/>
			<label class={label} for="entry-password">{m.field_password()}</label>
			<PasswordInput id="entry-password" bind:value={editing.content.password} />
			<label class={label} for="entry-url">{m.field_url()}</label>
			<input id="entry-url" class={field} bind:value={editing.content.url} />
			<label class={label} for="entry-notes">{m.field_notes()}</label>
			<textarea id="entry-notes" class={field} rows="3" bind:value={editing.content.notes}
			></textarea>
			<FieldsEditor id="entry" bind:fields={editedFields} />
			<fieldset class="mt-3">
				<legend class="text-sm font-medium">{m.vault_files()}</legend>
				<ul class="mt-1 text-sm">
					{#each (editing.content.attachments ?? []).filter((f) => !removedFiles.includes(f.id)) as file (file.id)}
						<li class="flex items-center gap-2">
							<span class="truncate">{file.name}</span>
							<span class="text-xs text-ink-3">{size(file.size)}</span>
							<button
								type="button"
								class="ml-auto rounded-md px-2 py-0.5 text-xs text-ink-3 hover:text-critical"
								onclick={() => (removedFiles = [...removedFiles, file.id])}
							>
								{m.vault_remove()}
							</button>
						</li>
					{/each}
					{#each pendingFiles as file, index (index)}
						<li class="flex items-center gap-2">
							<span class="truncate">{file.name}</span>
							<span class="text-xs text-ink-3">{size(file.size)}</span>
							<button
								type="button"
								class="ml-auto rounded-md px-2 py-0.5 text-xs text-ink-3 hover:text-critical"
								onclick={() => (pendingFiles = pendingFiles.filter((_, i) => i !== index))}
							>
								{m.vault_remove()}
							</button>
						</li>
					{/each}
				</ul>
				<label
					class="mt-1 inline-block cursor-pointer rounded-md px-2 py-1 text-sm text-ink-2 underline hover:text-ink"
				>
					{m.vault_add_file()}
					<input type="file" class="sr-only" multiple onchange={addFiles} />
				</label>
			</fieldset>
			<label class={label} for="entry-folder">{m.vault_folder()}</label>
			<select id="entry-folder" class={field} bind:value={editing.content.parent}>
				<option value={null}>{m.vault_title()}</option>
				{#each places as place (place.id)}
					<option value={place.id}>{place.path}</option>
				{/each}
			</select>
			<label class={label} for="entry-icon">{m.vault_icon()}</label>
			<IconPicker id="entry-icon" bind:value={editing.content.icon} />
			<div class="mt-5 flex justify-end gap-2">
				{#if editing.id}
					<button
						type="button"
						class="mr-auto rounded-lg px-3 py-1.5 text-sm text-critical hover:bg-surface-2"
						onclick={() => editing?.id && remove(editing.id)}
					>
						{m.catalog_delete()}
					</button>
				{/if}
				<button
					type="button"
					class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
					onclick={() => (editorOpen = false)}
				>
					{m.action_cancel()}
				</button>
				<button type="submit" class={primary}>{m.action_save()}</button>
			</div>
			{@render problem()}
		</form>
	{/if}
</Dialog>

<Dialog bind:open={kdbxOpen} title={kdbxMode === 'import' ? m.kdbx_import() : m.kdbx_export()}>
	{#if kdbxOpen && kdbxMode}
		<KdbxForm mode={kdbxMode} {busy} onimport={importPersonal} onexport={exportPersonal} />
		{#if kdbxResult}
			<p class="mt-3 text-sm" role="status">{kdbxResult}</p>
		{/if}
	{/if}
</Dialog>

<Dialog bind:open={folderOpen} title={m.vault_new_folder()}>
	{#if folderOpen}
		<form onsubmit={addFolder}>
			<label class="block text-sm font-medium" for="vault-folder-name">{m.field_name()}</label>
			<input
				id="vault-folder-name"
				class={field}
				required
				maxlength="200"
				bind:value={folderName}
			/>
			<button type="submit" class="{primary} mt-5">{m.action_create()}</button>
			{@render problem()}
		</form>
	{/if}
</Dialog>

<Dialog bind:open={passphraseOpen} title={m.vault_change_passphrase()}>
	{#if passphraseOpen}
		<form onsubmit={changePassphrase}>
			{@render passphraseFields()}
			<button type="submit" class="{primary} mt-5" disabled={busy}>{m.action_save()}</button>
			{@render problem()}
		</form>
	{/if}
</Dialog>

<Dialog bind:open={resetOpen} title={m.vault_reset()}>
	{#if resetOpen}
		<p class="text-sm">{m.vault_reset_confirm()}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={() => (resetOpen = false)}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={startOver}
			>
				{m.vault_reset()}
			</button>
		</div>
	{/if}
</Dialog>
