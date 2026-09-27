<script module lang="ts">
	import type { EditedField } from './FieldsEditor.svelte';

	/** An entry while it is edited (#193), personal or shared alike. */
	export interface Draft {
		title: string;
		username: string;
		/** A shared entry's stays if left empty. */
		password: string;
		url: string;
		notes: string;
		icon: number;
		fields: EditedField[];
		/** Comma-separated, as typed. */
		tags: string;
		/** `YYYY-MM-DD`; empty: never runs out. */
		expires: string;
		/** A new one-time password; empty keeps the stored one. */
		totp: string;
		removeTotp: boolean;
		/** Where it goes: `p:` and a personal folder (empty: the top), or `s:` and a collection. */
		place: string;
		newFiles: File[];
		/** Ids of files it had that go. */
		removedFiles: string[];
	}

	/** A place an entry may go, and the tree it belongs to. */
	export interface Place {
		value: string;
		label: string;
		group: 'personal' | 'shared';
	}

	/** The tags as the server and the personal vault keep them. */
	export const tagList = (text: string) =>
		[...new Set(text.split(',').map((tag) => tag.trim()))].filter(Boolean);
</script>

<script lang="ts">
	/**
	 * The entry editor (#193), with the tabs of KeePass' own: the entry, its
	 * one-time password, fields and files, where it lies and its history.
	 * Secrets of a shared entry are not shown here; left empty, they stay.
	 */
	import { untrack } from 'svelte';
	import { loadVersions, reveal, type Version } from '$lib/api/reveal';
	import { errorMessage } from '$lib/api/errors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import FieldsEditor from './FieldsEditor.svelte';
	import IconPicker from './IconPicker.svelte';
	import PasswordInput from './PasswordInput.svelte';
	import { parseTotp } from './totp';
	import type { EarlierContent } from './vault';

	/* eslint-disable svelte/no-unused-props -- the draft is copied whole into the form's state */
	let {
		draft: initial,
		shared,
		isNew,
		places,
		files,
		hasTotp,
		history = [],
		credentialId = null,
		mayReveal = false,
		busy = false,
		error = null,
		onsubmit,
		oncancel
	}: {
		draft: Draft;
		/** Whether the entry is a shared one now. */
		shared: boolean;
		isNew: boolean;
		places: Place[];
		files: { id: string; name: string; size: number }[];
		/** Whether a one-time password is stored already. */
		hasTotp: boolean;
		/** A personal entry's earlier states, newest first. */
		history?: EarlierContent[];
		/** A shared entry's id, for its versions. */
		credentialId?: string | null;
		/** Whether the shared entry's earlier passwords may be shown. */
		mayReveal?: boolean;
		busy?: boolean;
		error?: string | null;
		onsubmit: (draft: Draft) => void;
		oncancel: () => void;
	} = $props();
	/* eslint-enable svelte/no-unused-props */

	type Tab = 'entry' | 'advanced' | 'properties' | 'history';
	const TABS: { tab: Tab; label: () => string }[] = [
		{ tab: 'entry', label: m.vault_tab_entry },
		{ tab: 'advanced', label: m.vault_tab_advanced },
		{ tab: 'properties', label: m.vault_tab_properties },
		{ tab: 'history', label: m.vault_tab_history }
	];

	let draft = $state<Draft>(untrack(() => structuredClone($state.snapshot(initial)) as Draft));
	let tab = $state<Tab>('entry');
	let expiring = $state(untrack(() => !!initial.expires));
	/** Earlier passwords of a shared entry, by version, once shown. */
	let earlier = $state<Record<number, string>>({});
	let versions = $state<Version[] | null>(null);
	let historyError = $state<string | null>(null);

	const when = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
	const kilobytes = new Intl.NumberFormat(formatLocale(), { style: 'unit', unit: 'kilobyte' });
	const size = (bytes: number) => kilobytes.format(Math.max(1, Math.round(bytes / 1024)));
	/** Whether the typed one-time password reads as one, for a hint while typing. */
	const totpReads = $derived(!draft.totp.trim() || parseTotp(draft.totp.trim()) !== null);
	const movesAcross = $derived(!isNew && draft.place.startsWith('s:') !== shared);

	$effect(() => {
		if (tab !== 'history' || !credentialId || versions) return;
		loadVersions(credentialId).then((result) => {
			if (result.ok) versions = result.data;
			else historyError = errorMessage(result.code);
		});
	});

	async function showVersion(version: number) {
		if (!credentialId) return;
		const result = await reveal('credentials', credentialId, 'show', version);
		if (result.ok) earlier = { ...earlier, [version]: result.data.password ?? '' };
		else historyError = errorMessage(result.code);
	}

	async function restoreVersion(version: number) {
		if (!(version in earlier)) await showVersion(version);
		if (version in earlier) {
			draft.password = earlier[version];
			tab = 'entry';
		}
	}

	function restoreState(state: EarlierContent) {
		draft.title = state.title;
		draft.username = state.username;
		draft.password = state.password;
		draft.url = state.url;
		draft.notes = state.notes;
		tab = 'entry';
	}

	function addFiles(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		draft.newFiles = [...draft.newFiles, ...(input.files ?? [])];
		input.value = '';
	}

	/** What is missing, checked here: the browser cannot point into a hidden tab. */
	let missing = $state<string | null>(null);

	function submit(event: SubmitEvent) {
		event.preventDefault();
		missing = null;
		if (!draft.title.trim()) {
			tab = 'entry';
			missing = m.vault_title_missing();
			return;
		}
		if (expiring && !draft.expires) {
			tab = 'entry';
			missing = m.vault_expiry_missing();
			return;
		}
		if (!totpReads) {
			tab = 'advanced';
			return;
		}
		onsubmit({ ...$state.snapshot(draft), expires: expiring ? draft.expires : '' } as Draft);
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'text-sm font-medium';
	const row = 'grid items-start gap-x-4 gap-y-3 sm:grid-cols-[9rem_minmax(0,1fr)]';
	const small = 'rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2';
</script>

<form onsubmit={submit} autocomplete="off" novalidate class="flex flex-col gap-4">
	<div role="tablist" aria-label={m.vault_editor_tabs()} class="flex gap-1 border-b border-line">
		{#each TABS as item (item.tab)}
			<button
				type="button"
				role="tab"
				aria-selected={tab === item.tab}
				class="-mb-px border-b-2 px-3 py-2 text-sm aria-selected:border-accent aria-selected:font-semibold {tab ===
				item.tab
					? ''
					: 'border-transparent text-ink-2 hover:text-ink'}"
				onclick={() => (tab = item.tab)}
			>
				{item.label()}
			</button>
		{/each}
	</div>

	<div role="tabpanel" class="min-h-[22rem]">
		<div class={row} hidden={tab !== 'entry'}>
			<label class="{label} pt-3" for="entry-title">{m.field_title()}</label>
			<input id="entry-title" class={field} maxlength="200" bind:value={draft.title} />
			<label class="{label} pt-3" for="entry-username">{m.field_username()}</label>
			<input
				id="entry-username"
				class="{field} font-mono"
				maxlength="256"
				spellcheck="false"
				autocomplete="off"
				bind:value={draft.username}
			/>
			<label class="{label} pt-3" for="entry-password">{m.field_password()}</label>
			<div>
				<PasswordInput
					id="entry-password"
					describedby={shared && !isNew ? 'entry-password-keep' : undefined}
					bind:value={draft.password}
				/>
				{#if shared && !isNew}
					<p id="entry-password-keep" class="mt-1 text-xs text-ink-3">{m.vault_password_keep()}</p>
				{/if}
			</div>
			<label class="{label} pt-3" for="entry-url">{m.field_url()}</label>
			<input
				id="entry-url"
				class={field}
				maxlength="2000"
				spellcheck="false"
				bind:value={draft.url}
			/>
			<label class="{label} pt-3" for="entry-notes">{m.field_notes()}</label>
			<textarea id="entry-notes" class={field} rows="5" maxlength="10000" bind:value={draft.notes}
			></textarea>
			<span class="{label} pt-3">{m.vault_expires()}</span>
			<div class="mt-1 flex flex-wrap items-center gap-3">
				<label class="flex items-center gap-2 text-sm">
					<input type="checkbox" bind:checked={expiring} />
					{m.vault_expires_at()}
				</label>
				<input
					type="date"
					class="rounded-lg border border-line bg-page px-3 py-1.5 disabled:opacity-50"
					aria-label={m.vault_expires()}
					disabled={!expiring}
					bind:value={draft.expires}
				/>
			</div>
		</div>

		<div class="flex flex-col gap-5" hidden={tab !== 'advanced'}>
			<section class="flex flex-col gap-2 rounded-card border border-line p-4">
				<label class={label} for="entry-totp">{m.vault_totp()}</label>
				<p class="text-xs text-ink-3">{m.vault_totp_hint()}</p>
				<input
					id="entry-totp"
					class="{field} font-mono"
					spellcheck="false"
					autocomplete="off"
					placeholder={hasTotp && !draft.removeTotp ? m.vault_totp_stored() : ''}
					disabled={draft.removeTotp}
					bind:value={draft.totp}
				/>
				{#if !totpReads}
					<p class="text-xs text-critical" role="alert">{m.vault_totp_unreadable()}</p>
				{/if}
				{#if hasTotp}
					<label class="flex items-center gap-2 text-sm">
						<input type="checkbox" bind:checked={draft.removeTotp} />
						{m.vault_totp_remove()}
					</label>
				{/if}
			</section>
			<FieldsEditor id="entry" bind:fields={draft.fields} />
			<fieldset>
				<legend class={label}>{m.vault_files()}</legend>
				<ul class="mt-1 flex flex-col gap-1 text-sm">
					{#each files.filter((f) => !draft.removedFiles.includes(f.id)) as file (file.id)}
						<li class="flex items-center gap-2">
							<span class="truncate">{file.name}</span>
							<span class="text-xs text-ink-3">{size(file.size)}</span>
							<button
								type="button"
								class="ml-auto rounded-md px-2 py-0.5 text-xs text-ink-3 hover:text-critical"
								onclick={() => (draft.removedFiles = [...draft.removedFiles, file.id])}
							>
								{m.vault_remove()}
							</button>
						</li>
					{/each}
					{#each draft.newFiles as file, index (index)}
						<li class="flex items-center gap-2">
							<span class="truncate">{file.name}</span>
							<span class="text-xs text-ink-3">{size(file.size)}</span>
							<button
								type="button"
								class="ml-auto rounded-md px-2 py-0.5 text-xs text-ink-3 hover:text-critical"
								onclick={() => (draft.newFiles = draft.newFiles.filter((_, i) => i !== index))}
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
		</div>

		<div class={row} hidden={tab !== 'properties'}>
			<label class="{label} pt-3" for="entry-place">{m.vault_folder()}</label>
			<div>
				<select id="entry-place" class={field} bind:value={draft.place}>
					<optgroup label={m.vault_personal()}>
						{#each places.filter((p) => p.group === 'personal') as place (place.value)}
							<option value={place.value}>{place.label}</option>
						{/each}
					</optgroup>
					<optgroup label={m.vault_shared()}>
						{#each places.filter((p) => p.group === 'shared') as place (place.value)}
							<option value={place.value}>{place.label}</option>
						{/each}
					</optgroup>
				</select>
				{#if movesAcross}
					<p class="mt-1 text-xs text-ink-3">
						{shared ? m.vault_move_to_personal() : m.vault_move_to_shared()}
					</p>
				{/if}
			</div>
			<label class="{label} pt-3" for="entry-tags">{m.vault_tags()}</label>
			<div>
				<input id="entry-tags" class={field} maxlength="1100" bind:value={draft.tags} />
				<p class="mt-1 text-xs text-ink-3">{m.vault_tags_hint()}</p>
			</div>
			<label class="{label} pt-3" for="entry-icon">{m.vault_icon()}</label>
			<IconPicker id="entry-icon" bind:value={draft.icon} />
		</div>

		<div hidden={tab !== 'history'}>
			{#if historyError}
				<p class="text-sm text-critical" role="alert">{historyError}</p>
			{/if}
			{#if credentialId}
				{#if versions && versions.length > 0}
					<ul class="divide-y divide-line rounded-card border border-line text-sm">
						{#each versions as version (version.version)}
							<li class="flex items-center gap-3 px-3 py-2">
								<span class="w-40 tabular-nums">{when.format(new Date(version.created_at))}</span>
								<span class="flex-1 truncate font-mono" data-testid="earlier-password">
									{version.version in earlier ? earlier[version.version] : ''}
								</span>
								{#if mayReveal}
									<button type="button" class={small} onclick={() => showVersion(version.version)}>
										{m.vault_show()}
									</button>
									<button
										type="button"
										class={small}
										onclick={() => restoreVersion(version.version)}
									>
										{m.vault_restore()}
									</button>
								{/if}
							</li>
						{/each}
					</ul>
				{:else if versions}
					<p class="text-sm text-ink-3">{m.vault_history_none()}</p>
				{/if}
			{:else if history.length > 0}
				<ul class="divide-y divide-line rounded-card border border-line text-sm">
					{#each history as state (state.at)}
						<li class="flex items-center gap-3 px-3 py-2">
							<span class="w-40 tabular-nums">{when.format(new Date(state.at))}</span>
							<span class="flex-1 truncate">{state.title}</span>
							<span class="font-mono" data-testid="earlier-password">{state.password}</span>
							<button type="button" class={small} onclick={() => restoreState(state)}>
								{m.vault_restore()}
							</button>
						</li>
					{/each}
				</ul>
			{:else}
				<p class="text-sm text-ink-3">{m.vault_history_none()}</p>
			{/if}
		</div>
	</div>

	{#if missing ?? error}
		<p class="text-sm text-critical" role="alert">{missing ?? error}</p>
	{/if}
	<div class="flex justify-end gap-2 border-t border-line pt-4">
		<button
			type="button"
			class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
			onclick={oncancel}
		>
			{m.action_cancel()}
		</button>
		<button
			type="submit"
			class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
			disabled={busy || !totpReads}
		>
			{isNew ? m.action_create() : m.action_save()}
		</button>
	</div>
</form>
