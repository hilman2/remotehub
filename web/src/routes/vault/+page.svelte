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
	import { getLocale } from '$lib/i18n';
	import { frequent, queryKey, rank, remember, type Pick } from '$lib/search/rank';
	import Dialog from '$lib/components/Dialog.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';
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
		type StoredVault,
		type UnlockKind
	} from '$lib/vault/vault';

	type Stage = 'loading' | 'setup' | 'recovery' | 'locked' | 'open';

	const MIN_PASSPHRASE = 12;
	const KIND_LABELS: Record<UnlockKind, () => string> = {
		passkey: m.vault_kind_passkey,
		passphrase: m.vault_kind_passphrase,
		recovery: m.vault_kind_recovery
	};
	const EMPTY: EntryContent = { title: '', username: '', password: '', url: '', notes: '' };

	let stage = $state<Stage>('loading');
	let vault = $state<StoredVault | null>(null);
	// The vault key: in memory only, gone when the page is.
	let key: CryptoKey | null = null;
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

	/** Readable entries, best first; entries that do not open, last. */
	const shown = $derived.by(() => {
		const readable = entries.filter((entry) => entry.content);
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
		return [
			...[...readable].sort((a, b) => place(a) - place(b)),
			...entries.filter((entry) => !entry.content)
		];
	});

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
		if (key) entries = await readEntries(key, vault);
		else stage = vault.unlocks.length === 0 ? 'setup' : 'locked';
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
		key = result.key;
		shownRecovery = result.recovery;
		passphrase = passphraseAgain = '';
		stage = 'recovery';
	}

	async function opened(unlocked: CryptoKey | null) {
		if (!unlocked || !vault) {
			error = m.vault_wrong();
			return;
		}
		key = unlocked;
		error = null;
		passphrase = recoveryText = '';
		[entries, picks] = await Promise.all([readEntries(key, vault), readPicks(key, vault)]);
		stage = 'open';
	}

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
		const unlocked = useRecovery
			? await unlockWithRecovery(vault, recoveryText)
			: await unlockWithPassphrase(vault, passphrase);
		busy = false;
		await opened(unlocked);
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
		key = null;
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
	}

	function edit(entry: Entry | null) {
		if (entry) used(entry);
		editing = { id: entry?.id ?? null, content: { ...(entry?.content ?? EMPTY) } };
		error = null;
		editorOpen = true;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (!key || !editing) return;
		const result = await saveEntry(key, editing.id, editing.content);
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
		key = null;
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
		<button type="button" class="{button} ml-auto" onclick={() => edit(null)}>
			<Plus size={16} aria-hidden="true" />
			{m.vault_new_entry()}
		</button>
	</div>
	{#if !editorOpen}{@render problem()}{/if}
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
						<div class="min-w-0">
							<p class="font-medium">{entry.content.title}</p>
							<p class="truncate text-ink-2">
								{entry.content.username}{entry.content.url ? ` · ${entry.content.url}` : ''}
							</p>
							{#if revealed === entry.id}
								<p class="mt-1 font-mono break-all">{entry.content.password}</p>
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
			<input
				id="entry-password"
				type="password"
				class={field}
				autocomplete="new-password"
				bind:value={editing.content.password}
			/>
			<label class={label} for="entry-url">{m.field_url()}</label>
			<input id="entry-url" class={field} bind:value={editing.content.url} />
			<label class={label} for="entry-notes">{m.field_notes()}</label>
			<textarea id="entry-notes" class={field} rows="3" bind:value={editing.content.notes}
			></textarea>
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
