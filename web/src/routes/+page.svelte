<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Clock from '@lucide/svelte/icons/clock';
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import FolderClosed from '@lucide/svelte/icons/folder-closed';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Lock from '@lucide/svelte/icons/lock';
	import PanelLeftOpen from '@lucide/svelte/icons/panel-left-open';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { resolve } from '$app/paths';
	import {
		allows,
		createCredential,
		createDevice,
		createFolder,
		deleteCredential,
		deleteDevice,
		deleteFolder,
		loadTree,
		renameFolder,
		resetHostKey,
		setFolderOpen,
		updateCredential,
		updateDevice,
		type Credential,
		type CredentialInput,
		type Device,
		type DeviceInput,
		type Folder,
		type ObjectKind,
		type Role,
		type Tree
	} from '$lib/api/catalog';
	import type { ApiResult } from '$lib/api/client';
	import { loadConnectors, type Connector } from '$lib/api/connectors';
	import { createRequest, requestableRoles } from '$lib/api/requests';
	import { loadPicks, savePick } from '$lib/api/search';
	import { tabs } from '$lib/session/tabs.svelte';
	import ProtocolChip from '$lib/catalog/ProtocolChip.svelte';
	import { catalogItems, pickKey, type Hit } from '$lib/search/catalog';
	import { frequent, queryKey, rank, remember, type Pick } from '$lib/search/rank';
	import RequestForm from '$lib/catalog/RequestForm.svelte';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import CredentialForm from '$lib/catalog/CredentialForm.svelte';
	import DeviceForm from '$lib/catalog/DeviceForm.svelte';
	import FolderNodeView from '$lib/catalog/FolderNodeView.svelte';
	import Grants from '$lib/catalog/Grants.svelte';
	import {
		AUTH_MODE_LABELS,
		CREDENTIAL_KIND_LABELS,
		PROTOCOL_LABELS,
		ROLE_LABELS,
		keyboardLayoutLabel
	} from '$lib/catalog/labels';
	import { nest, pathTo } from '$lib/catalog/tree';
	import Dialog from '$lib/components/Dialog.svelte';
	import SettingsMenu, { type MenuItem } from '$lib/components/SettingsMenu.svelte';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { SvelteSet } from 'svelte/reactivity';

	type Selection = { kind: ObjectKind; id: string };
	type Open =
		| { type: 'folder'; parent: string | null; folder: Folder | null }
		| { type: 'device'; folderId: string; device: Device | null }
		| { type: 'credential'; folderId: string; credential: Credential | null }
		| { type: 'grants'; kind: ObjectKind; id: string; name: string }
		| { type: 'delete'; kind: ObjectKind; id: string; name: string }
		| { type: 'request'; kind: ObjectKind; id: string; name: string; role: Role };

	let tree = $state<Tree | null>(null);
	let connectors = $state<Connector[]>([]);
	let query = $state('');
	let selected = $state<Selection | null>(null);
	/** Folders open in the tree; the server keeps them per user (#83). */
	const openFolders = new SvelteSet<string>();
	let saving: Promise<unknown> = Promise.resolve();
	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let error = $state<string | null>(null);
	let folderName = $state('');
	/** The object whose access request was just sent, for the notice. */
	let requested = $state<string | null>(null);

	/** What this user picked before, for ranking (#81). */
	let picks = $state<Pick[]>([]);
	/** The result Enter picks. */
	let active = $state(0);

	const roots = $derived(tree ? nest(tree, getLocale()) : []);
	const searching = $derived(queryKey(query).length > 0);
	const items = $derived(tree ? catalogItems(tree) : []);
	const results = $derived(
		searching ? rank(items, query, picks, Date.now(), getLocale()).slice(0, 50) : []
	);
	const favourites = $derived.by(() => {
		const byKey = new Map(items.map((entry) => [entry.key, entry.item]));
		return frequent(picks, Date.now(), 12)
			.flatMap((key) => byKey.get(key) ?? [])
			.filter((hit) => hit.kind === 'device')
			.slice(0, 5);
	});

	$effect(() => {
		// A new query starts at its best result.
		void query;
		active = 0;
	});

	/** Selects `hit` and remembers it was picked for the current query. */
	function choose(kind: ObjectKind, id: string) {
		selected = { kind, id };
		const key = pickKey(kind, id);
		picks = remember(picks, key, query, Date.now());
		savePick(key, queryKey(query));
	}

	function onSearchKey(event: KeyboardEvent) {
		if (!searching || results.length === 0) return;
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			const step = event.key === 'ArrowDown' ? 1 : -1;
			active = (active + step + results.length) % results.length;
		} else if (event.key === 'Enter') {
			event.preventDefault();
			const hit = results[active];
			choose(hit.kind, hit.id);
		}
	}
	const folder = $derived(
		selected?.kind === 'folder' ? tree?.folders.find((f) => f.id === selected?.id) : undefined
	);
	const device = $derived(
		selected?.kind === 'device' ? tree?.devices.find((d) => d.id === selected?.id) : undefined
	);
	const credential = $derived(
		selected?.kind === 'credential'
			? tree?.credentials.find((c) => c.id === selected?.id)
			: undefined
	);

	async function load() {
		await saving;
		const [result, sites, picked] = await Promise.all([loadTree(), loadConnectors(), loadPicks()]);
		if (result.ok) {
			tree = result.data;
			// The server's word; a toggle meanwhile is stored by then anyway.
			openFolders.clear();
			for (const id of result.data.open) openFolders.add(id);
		} else error = errorMessage(result.code);
		if (sites.ok) connectors = sites.data;
		if (picked.ok) picks = picked.data;
	}

	const connectorOf = (device: Device) => connectors.find((c) => c.id === device.connector_id);
	const path = (folderId: string | null) =>
		tree
			? pathTo(tree, folderId)
					.map((f) => f.name)
					.join(' / ')
			: '';

	$effect(() => {
		load();
	});

	function show(next: Open) {
		error = null;
		open = next;
		if (next.type === 'folder') folderName = next.folder?.name ?? '';
		dialogOpen = true;
	}

	/** Runs a change; on success reloads the tree and closes the dialog. */
	async function run(change: Promise<ApiResult<unknown>>, select?: (data: unknown) => void) {
		const result = await change;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		dialogOpen = false;
		error = null;
		select?.(result.data);
		await load();
	}

	/** Selects what was just created, and opens the folder it went into. */
	const created = (kind: ObjectKind, parent: string | null) => (data: unknown) => {
		const id = (data as { id?: string } | undefined)?.id;
		if (id) selected = { kind, id };
		if (parent) setOpen(parent, true);
	};

	function saveFolder(event: SubmitEvent) {
		event.preventDefault();
		if (open?.type !== 'folder') return;
		const target = open;
		run(
			target.folder
				? renameFolder(target.folder.id, folderName)
				: createFolder(target.parent, folderName),
			target.folder ? undefined : created('folder', target.parent)
		);
	}

	function saveDevice(input: DeviceInput) {
		if (open?.type !== 'device') return;
		const target = open;
		run(
			target.device ? updateDevice(target.device.id, input) : createDevice(input),
			target.device ? undefined : created('device', input.folder_id)
		);
	}

	function saveCredential(input: CredentialInput) {
		if (open?.type !== 'credential') return;
		const target = open;
		run(
			target.credential ? updateCredential(target.credential.id, input) : createCredential(input),
			target.credential ? undefined : created('credential', input.folder_id)
		);
	}

	function remove() {
		if (open?.type !== 'delete') return;
		const { kind, id } = open;
		const removal =
			kind === 'folder'
				? deleteFolder(id)
				: kind === 'device'
					? deleteDevice(id)
					: deleteCredential(id);
		run(removal, () => (selected = null));
	}

	function sendRequest(role: Role, minutes: number, reason: string) {
		if (open?.type !== 'request') return;
		const { kind, id } = open;
		run(createRequest({ kind, id }, role, minutes, reason), () => (requested = id));
	}

	function setOpen(id: string, open: boolean) {
		if (open === openFolders.has(id)) return;
		if (open) openFolders.add(id);
		else openFolders.delete(id);
		// A preference: if it is not stored, the tree still works. The next
		// load waits for it, or it would read the tree from before.
		saving = Promise.all([saving, setFolderOpen(id, open)]);
	}

	const toggle = (id: string) => setOpen(id, !openFolders.has(id));

	/** A double-click on a device in a list connects, where the user may. */
	function connectTo(id: string) {
		const target = tree?.devices.find((d) => d.id === id);
		if (target && allows(target.role, 'connect')) tabs.open(target);
	}

	/** Behind the gear: changing, permissions and deleting, as far as `role` allows. */
	function settings(
		kind: ObjectKind,
		id: string,
		name: string,
		role: Role | null,
		edit: MenuItem
	): MenuItem[] {
		// Folders are changed by who manages them, the rest by who may edit it.
		const change = allows(role, kind === 'folder' ? 'manage' : 'edit');
		const permissions: MenuItem = {
			label: m.catalog_permissions(),
			icon: ShieldCheck,
			onselect: () => show({ type: 'grants', kind, id, name })
		};
		const remove: MenuItem = {
			label: m.catalog_delete(),
			icon: Trash2,
			danger: true,
			onselect: () => show({ type: 'delete', kind, id, name })
		};
		return [
			...(change ? [edit] : []),
			...(allows(role, 'manage') ? [permissions] : []),
			...(change ? [remove] : [])
		];
	}

	const dialogTitle = $derived.by(() => {
		switch (open?.type) {
			case 'folder':
				return open.folder
					? m.catalog_rename()
					: open.parent
						? m.catalog_new_subfolder()
						: m.catalog_new_folder();
			case 'device':
				return open.device ? m.catalog_edit() : m.catalog_new_device();
			case 'credential':
				return open.credential ? m.catalog_edit() : m.catalog_new_credential();
			case 'grants':
				return m.grants_title({ name: open.name });
			case 'delete':
				return m.catalog_delete();
			case 'request':
				return m.request_title({ name: open.name });
			default:
				return '';
		}
	});

	const button =
		'inline-flex h-10 items-center gap-2 rounded-xl border border-line-strong bg-surface px-3.5 text-sm hover:bg-surface-2';
	const card = 'flex flex-col gap-3 rounded-card border border-line bg-surface p-5';
</script>

{#snippet requestAccess(kind: ObjectKind, id: string, name: string, role: Role | null)}
	{#if role && requestableRoles(role).length > 0}
		<button
			type="button"
			class={button}
			onclick={() => show({ type: 'request', kind, id, name, role })}
		>
			<Clock size={16} aria-hidden="true" />
			{m.request_access()}
		</button>
	{/if}
{/snippet}

{#snippet requestedNotice(id: string)}
	{#if requested === id}
		<p class="flex items-center gap-2 text-sm" role="status">
			<CircleCheck size={16} class="text-ok" aria-hidden="true" />
			{m.request_sent()}
			<a class="underline" href={resolve('/requests')}>{m.nav_requests()}</a>
		</p>
	{/if}
{/snippet}

{#snippet result(hit: Hit, highlighted: boolean)}
	{@const isSelected = selected?.kind === hit.kind && selected.id === hit.id}
	<li>
		<button
			type="button"
			class="flex w-full min-w-0 flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left text-sm hover:bg-surface-2 data-[active=true]:bg-surface-2 data-[active=true]:ring-1 data-[active=true]:ring-line-strong"
			data-active={highlighted || isSelected}
			aria-current={isSelected ? 'true' : undefined}
			onclick={() => choose(hit.kind, hit.id)}
			ondblclick={() => hit.kind === 'device' && connectTo(hit.id)}
		>
			<span class="flex w-full min-w-0 items-center gap-2.5">
				{#if hit.protocol}
					<ProtocolChip protocol={hit.protocol} />
				{:else if hit.kind === 'credential'}
					<KeyRound size={15} class="shrink-0 text-warning" aria-hidden="true" />
				{:else}
					<FolderClosed size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
				{/if}
				<span class="truncate text-ink">{hit.name}</span>
				<span class="ml-auto truncate font-mono text-xs text-ink-3">{hit.detail}</span>
			</span>
			{#if hit.where}
				<span class="truncate pl-0.5 text-xs text-ink-3">{hit.where}</span>
			{/if}
		</button>
	</li>
{/snippet}

{#snippet heading(where: string, name: string)}
	<div class="flex flex-col gap-3">
		<p class="text-sm text-ink-3">{where}</p>
		<h2 class="text-4xl leading-none font-semibold break-words sm:text-5xl">{name}</h2>
	</div>
{/snippet}

{#snippet pin(fingerprint: string | null, role: Role)}
	{#if fingerprint}
		<p class="flex items-center gap-2 font-medium">
			<ShieldCheck size={16} class="text-ok" aria-hidden="true" />
			{m.device_pinned()}
		</p>
		<p class="font-mono text-xs leading-relaxed break-all text-ink-2">{fingerprint}</p>
		{#if allows(role, 'edit') && device}
			<button
				type="button"
				class="mt-auto self-start text-sm text-ink-2 underline hover:text-ink"
				onclick={() => device && run(resetHostKey(device.id))}
			>
				{m.device_forget_host_key()}
			</button>
		{/if}
	{:else}
		<p class="text-ink-2">{m.device_host_key_pending()}</p>
	{/if}
{/snippet}

{#if tabs.active !== null}
	<!-- A session fills the page (see +layout.svelte); the devices wait in a strip. -->
	<div class="flex w-12 flex-1 flex-col items-center gap-4 border-r border-line bg-sunken py-3">
		<button
			type="button"
			class="flex size-9 items-center justify-center rounded-lg border border-line-strong text-ink-2 hover:bg-surface-2 hover:text-ink"
			title={m.session_show_devices()}
			onclick={() => (tabs.active = null)}
		>
			<PanelLeftOpen size={17} aria-hidden="true" />
			<span class="sr-only">{m.session_show_devices()}</span>
		</button>
		<span class="rotate-180 eyebrow [writing-mode:vertical-rl]" aria-hidden="true">
			{m.devices_title()}
		</span>
	</div>
{/if}

<div
	class="flex min-h-0 flex-1 flex-col overflow-y-auto lg:flex-row lg:overflow-hidden"
	class:hidden={tabs.active !== null}
>
	<aside
		class="flex shrink-0 flex-col gap-4 border-b border-line bg-sunken px-3 py-5 lg:w-84 lg:overflow-y-auto lg:border-r lg:border-b-0"
	>
		<div class="flex items-center gap-2 px-2">
			<h1 class="eyebrow">{m.devices_title()}</h1>
			{#if tree?.may_create_top_level}
				<button
					type="button"
					class="ml-auto flex size-8 items-center justify-center rounded-lg border border-line-strong text-ink-2 hover:bg-surface-2 hover:text-ink"
					title={m.catalog_new_folder()}
					onclick={() => show({ type: 'folder', parent: null, folder: null })}
				>
					<FolderPlus size={15} aria-hidden="true" />
					<span class="sr-only">{m.catalog_new_folder()}</span>
				</button>
			{/if}
		</div>
		<label class="relative block">
			<span class="sr-only">{m.catalog_search()}</span>
			<Search size={16} class="absolute top-3 left-3 text-ink-3" aria-hidden="true" />
			<input
				type="search"
				class="h-10 w-full rounded-xl border border-line-strong bg-page pr-3 pl-9 text-sm"
				placeholder={m.catalog_search()}
				bind:value={query}
				onkeydown={onSearchKey}
			/>
		</label>
		{#if searching}
			{#if results.length === 0}
				<p class="px-2 text-sm text-ink-2">{m.catalog_no_match()}</p>
			{:else}
				<ul aria-label={m.search_results()} class="flex flex-col gap-0.5">
					{#each results as hit, index (pickKey(hit.kind, hit.id))}
						{@render result(hit, index === active)}
					{/each}
				</ul>
			{/if}
		{:else if tree && tree.folders.length > 0}
			{#if favourites.length > 0}
				<div class="flex flex-col gap-1">
					<h2 class="px-2 eyebrow">{m.search_frequent()}</h2>
					<ul class="flex flex-col gap-0.5">
						{#each favourites as hit (hit.id)}
							{@render result(hit, false)}
						{/each}
					</ul>
				</div>
			{/if}
			<nav aria-label={m.devices_title()}>
				<ul role="tree" aria-label={m.devices_title()} class="flex flex-col gap-0.5">
					{#each roots as node (node.folder.id)}
						<FolderNodeView
							{node}
							{selected}
							expanded={(id) => openFolders.has(id)}
							onselect={choose}
							ontoggle={toggle}
							onopen={connectTo}
						/>
					{/each}
				</ul>
			</nav>
		{/if}
	</aside>

	<section
		class="flex min-w-0 flex-1 flex-col gap-7 px-5 py-8 sm:px-10 lg:overflow-y-auto lg:py-10"
		aria-live="polite"
	>
		{#if error && !dialogOpen}
			<p class="flex items-center gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="text-critical" aria-hidden="true" />
				{error}
			</p>
		{/if}

		{#if tree && tree.folders.length === 0}
			<p class="font-display text-2xl text-ink-2">{m.devices_empty_title()}</p>
		{:else if folder}
			<div class="flex flex-wrap items-end gap-6">
				<div class="min-w-0 flex-1">{@render heading(path(folder.parent_id), folder.name)}</div>
				<div class="flex items-center gap-2">
					{@render requestAccess('folder', folder.id, folder.name, folder.role)}
					<SettingsMenu
						label={m.catalog_settings()}
						items={settings('folder', folder.id, folder.name, folder.role, {
							label: m.catalog_rename(),
							icon: Pencil,
							onselect: () => show({ type: 'folder', parent: folder.parent_id, folder })
						})}
					/>
				</div>
			</div>
			{#if folder.role}
				<div class="flex flex-wrap gap-2">
					<span class="chip">{m.catalog_access({ role: ROLE_LABELS[folder.role]() })}</span>
				</div>
			{/if}
			{#if allows(folder.role, 'edit')}
				<div class="flex flex-wrap gap-2">
					<button
						type="button"
						class={button}
						onclick={() => show({ type: 'device', folderId: folder.id, device: null })}
					>
						<Plus size={16} aria-hidden="true" />
						{m.catalog_new_device()}
					</button>
					<button
						type="button"
						class={button}
						onclick={() => show({ type: 'credential', folderId: folder.id, credential: null })}
					>
						<KeyRound size={16} aria-hidden="true" />
						{m.catalog_new_credential()}
					</button>
					{#if allows(folder.role, 'manage')}
						<button
							type="button"
							class={button}
							onclick={() => show({ type: 'folder', parent: folder.id, folder: null })}
						>
							<FolderPlus size={16} aria-hidden="true" />
							{m.catalog_new_subfolder()}
						</button>
					{/if}
				</div>
			{/if}
			{@render requestedNotice(folder.id)}
		{:else if device}
			<div class="flex flex-wrap items-end gap-6">
				<div class="flex min-w-0 flex-1 flex-col gap-4">
					{@render heading(path(device.folder_id), device.name)}
					<div class="flex flex-wrap gap-2">
						<span class="chip font-mono">{device.host}:{device.port}</span>
						<span class="chip">{PROTOCOL_LABELS[device.protocol]()}</span>
						{#if device.connector_id}
							{@const connector = connectorOf(device)}
							<span class="chip">
								{#if connector?.online}
									<span class="size-2 rounded-full bg-ok" aria-hidden="true"></span>
								{:else}
									<TriangleAlert size={13} class="text-warning" aria-hidden="true" />
								{/if}
								<span>
									{connector?.name ?? ''} · {connector?.online
										? m.connector_online()
										: m.connector_offline()}
								</span>
							</span>
						{/if}
						<span class="chip">{m.catalog_access({ role: ROLE_LABELS[device.role]() })}</span>
					</div>
				</div>
				<div class="flex flex-col items-end gap-2">
					<div class="flex items-center gap-2">
						{@render requestAccess('device', device.id, device.name, device.role)}
						<SettingsMenu
							label={m.catalog_settings()}
							items={settings('device', device.id, device.name, device.role, {
								label: m.catalog_edit(),
								icon: Pencil,
								onselect: () => show({ type: 'device', folderId: device.folder_id, device })
							})}
						/>
						{#if allows(device.role, 'connect')}
							<button
								type="button"
								class="inline-flex h-14 items-center rounded-2xl bg-accent px-8 font-display text-lg font-semibold text-accent-ink hover:brightness-110"
								onclick={() => device && tabs.open(device)}
							>
								{m.device_connect()}
							</button>
						{/if}
					</div>
					{#if allows(device.role, 'connect')}
						<!-- A window of its own, e.g. for a second screen. -->
						<a
							href={resolve('/connect/[id]', { id: device.id })}
							target="_blank"
							rel="noopener"
							class="inline-flex items-center gap-1.5 text-sm text-ink-2 underline hover:text-ink"
						>
							<ExternalLink size={14} aria-hidden="true" />
							{m.session_new_window()}
						</a>
					{/if}
				</div>
			</div>

			<div class="grid gap-4 md:grid-cols-3">
				<section class={card}>
					<h3 class="eyebrow">{m.field_auth_mode()}</h3>
					<p class="text-lg font-semibold">
						{device.credential_id
							? (tree?.credentials.find((c) => c.id === device.credential_id)?.name ?? '')
							: AUTH_MODE_LABELS[device.auth_mode]()}
					</p>
					{#if device.credential_id}
						<p class="mt-auto flex items-center gap-2 text-sm text-ink-2">
							<Lock size={14} aria-hidden="true" />
							{AUTH_MODE_LABELS[device.auth_mode]()}
						</p>
					{/if}
				</section>
				{#if device.protocol === 'ssh'}
					<section class={card}>
						<h3 class="eyebrow">{m.device_host_key()}</h3>
						{@render pin(device.host_key_fingerprint, device.role)}
					</section>
				{:else if device.protocol === 'rdp' || device.protocol === 'https'}
					<section class={card}>
						<h3 class="eyebrow">{m.device_certificate()}</h3>
						{@render pin(device.certificate_fingerprint, device.role)}
					</section>
				{/if}
				{#if device.protocol === 'rdp'}
					<section class={card}>
						<h3 class="eyebrow">{m.field_keyboard_layout()}</h3>
						<p class="text-lg font-semibold">
							{device.keyboard_layout
								? keyboardLayoutLabel(device.keyboard_layout, getLocale())
								: m.keyboard_layout_default()}
						</p>
					</section>
				{/if}
				{#if device.description}
					<section class="{card} md:col-span-3">
						<h3 class="eyebrow">{m.field_description()}</h3>
						<p class="whitespace-pre-line">{device.description}</p>
					</section>
				{/if}
			</div>

			{@render requestedNotice(device.id)}
		{:else if credential}
			<div class="flex flex-wrap items-end gap-6">
				<div class="min-w-0 flex-1">
					{@render heading(path(credential.folder_id), credential.name)}
				</div>
				<div class="flex items-center gap-2">
					{@render requestAccess('credential', credential.id, credential.name, credential.role)}
					<SettingsMenu
						label={m.catalog_settings()}
						items={settings('credential', credential.id, credential.name, credential.role, {
							label: m.catalog_edit(),
							icon: Pencil,
							onselect: () =>
								show({ type: 'credential', folderId: credential.folder_id, credential })
						})}
					/>
				</div>
			</div>
			<div class="flex flex-wrap gap-2">
				<span class="chip font-mono">
					{credential.domain ? `${credential.domain}\\${credential.username}` : credential.username}
				</span>
				<span class="chip">{CREDENTIAL_KIND_LABELS[credential.kind]()}</span>
				<span class="chip">{m.catalog_access({ role: ROLE_LABELS[credential.role]() })}</span>
				<span class="chip">{m.credential_version({ version: credential.version })}</span>
			</div>
			<div class="grid gap-4 md:grid-cols-3">
				<section class={card}>
					<h3 class="eyebrow">
						{credential.kind === 'ssh_key' ? m.credential_key() : m.field_password()}
					</h3>
					<p class="flex items-center gap-2 font-medium">
						<Lock size={16} class="text-ok" aria-hidden="true" />
						{m.credential_hidden()}
					</p>
					{#if credential.kind === 'ssh_key'}
						<p class="font-mono text-xs text-ink-2">{credential.key_algorithm}</p>
						<p class="font-mono text-xs leading-relaxed break-all text-ink-2">
							{credential.key_fingerprint}
						</p>
					{/if}
				</section>
				{#if credential.kind === 'ssh_key'}
					<section class={card}>
						<h3 class="eyebrow">{m.credential_certificate()}</h3>
						<p class="text-lg font-semibold">
							{credential.has_certificate
								? m.credential_certificate_yes()
								: m.credential_certificate_no()}
						</p>
					</section>
				{/if}
			</div>
			{@render requestedNotice(credential.id)}
		{:else if tree}
			<p class="font-display text-2xl text-ink-3">{m.catalog_select_hint()}</p>
		{/if}
	</section>
</div>

<Dialog bind:open={dialogOpen} title={dialogTitle}>
	<!-- Mounted per opening: no form keeps the state, or a key, of the last one. -->
	{#if dialogOpen}
		{#key open}
			{#if open?.type === 'folder'}
				<form onsubmit={saveFolder}>
					<label class="block text-sm font-medium" for="folder-name">{m.field_name()}</label>
					<input
						id="folder-name"
						class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
						required
						maxlength="200"
						bind:value={folderName}
					/>
					<div class="mt-5 flex justify-end gap-2">
						<button
							type="button"
							class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
							onclick={() => (dialogOpen = false)}
						>
							{m.action_cancel()}
						</button>
						<button
							type="submit"
							class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
						>
							{open.folder ? m.action_save() : m.action_create()}
						</button>
					</div>
				</form>
			{:else if open?.type === 'device' && tree}
				<DeviceForm
					folderId={open.folderId}
					device={open.device}
					credentials={tree.credentials}
					{connectors}
					onsubmit={saveDevice}
					oncancel={() => (dialogOpen = false)}
				/>
			{:else if open?.type === 'credential'}
				<CredentialForm
					folderId={open.folderId}
					credential={open.credential}
					onsubmit={saveCredential}
					oncancel={() => (dialogOpen = false)}
				/>
			{:else if open?.type === 'grants'}
				<Grants kind={open.kind} id={open.id} />
			{:else if open?.type === 'request'}
				<RequestForm
					held={open.role}
					onsubmit={sendRequest}
					oncancel={() => (dialogOpen = false)}
				/>
			{:else if open?.type === 'delete'}
				<p class="text-sm">{m.catalog_delete_confirm({ name: open.name })}</p>
				<div class="mt-5 flex justify-end gap-2">
					<button
						type="button"
						class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
						onclick={() => (dialogOpen = false)}
					>
						{m.action_cancel()}
					</button>
					<button
						type="button"
						class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
						onclick={remove}
					>
						{m.catalog_delete()}
					</button>
				</div>
			{/if}
		{/key}
	{/if}
	{#if error && dialogOpen}
		<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
</Dialog>
