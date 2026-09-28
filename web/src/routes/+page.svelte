<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Clock from '@lucide/svelte/icons/clock';
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import FolderClosed from '@lucide/svelte/icons/folder-closed';
	import FolderInput from '@lucide/svelte/icons/folder-input';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
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
		createDevice,
		createFolder,
		deleteDevice,
		deleteFolder,
		loadTree,
		moveDevice,
		moveFolder,
		resetHostKey,
		setFolderOpen,
		updateDevice,
		updateFolder,
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
	import { queryKey, rank, remember, type Pick } from '$lib/search/rank';
	import RequestForm from '$lib/catalog/RequestForm.svelte';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import DeviceForm from '$lib/catalog/DeviceForm.svelte';
	import FolderNodeView from '$lib/catalog/FolderNodeView.svelte';
	import Grants from '$lib/catalog/Grants.svelte';
	import Journal from '$lib/catalog/Journal.svelte';
	import Keywords from '$lib/catalog/Keywords.svelte';
	import ProfilesView from '$lib/catalog/ProfilesView.svelte';
	import RevealSecret from '$lib/catalog/RevealSecret.svelte';
	import AskCustomer from '$lib/connectors/AskCustomer.svelte';
	import DeviceAccess from '$lib/connectors/DeviceAccess.svelte';
	import {
		AUTH_MODE_LABELS,
		PROTOCOL_LABELS,
		ROLE_LABELS,
		keyboardLayoutLabel
	} from '$lib/catalog/labels';
	import {
		folderConnector,
		movePlaces,
		nest,
		pathTo,
		profilesWithin,
		takes,
		type Dragged,
		type Mover
	} from '$lib/catalog/tree';
	import Dialog from '$lib/components/Dialog.svelte';
	import SettingsMenu, { type MenuItem } from '$lib/components/SettingsMenu.svelte';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { untrack } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';

	type Selection = { kind: ObjectKind; id: string };
	type Open =
		| { type: 'folder'; parent: string | null; folder: Folder | null }
		| { type: 'device'; folderId: string; device: Device | null }
		| { type: 'grants'; kind: ObjectKind; id: string; name: string }
		| { type: 'delete'; kind: ObjectKind; id: string; name: string }
		| { type: 'request'; kind: ObjectKind; id: string; name: string; role: Role }
		| { type: 'move'; item: Dragged; name: string };

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
	/** The folder's own site connector (#176); empty: its parent's. */
	let folderConnectorId = $state('');
	/** The object whose access request was just sent, for the notice. */
	let requested = $state<string | null>(null);

	/** What this user picked before, for ranking (#81). */
	let picks = $state<Pick[]>([]);
	/** The result Enter picks. */
	let active = $state(0);

	const roots = $derived(tree ? nest(tree, getLocale()) : []);
	const searching = $derived(queryKey(query).length > 0);
	const items = $derived(tree ? catalogItems(tree) : []);
	const results = $derived.by(() => {
		if (!searching) return [];
		// A pick counts from the next query on, so the results do not move under
		// the pointer between the clicks of a double-click (#210).
		const counted = untrack(() => picks);
		return rank(items, query, counted, Date.now(), getLocale()).slice(0, 50);
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

	/** The connections or the login profiles (#192). */
	let area = $state<'connections' | 'profiles'>('connections');
	/** The login profile shown in the profiles area. */
	let profileId = $state<string | null>(null);

	/** Shows a device in the connections area, with the folders above it open. */
	function showDevice(id: string) {
		const target = tree?.devices.find((d) => d.id === id);
		if (!target || !tree) return;
		area = 'connections';
		selected = { kind: 'device', id };
		for (const above of pathTo(tree, target.folder_id)) setOpen(above.id, true);
	}

	function showProfile(id: string) {
		profileId = id;
		area = 'profiles';
	}

	const connectorOf = (device: Device) => connectors.find((c) => c.id === device.reached_through);
	/** The connector a folder passes on to what is in it (#176). */
	const passedOn = (folderId: string | null) => {
		const id = tree ? folderConnector(tree, folderId) : null;
		return id ? (connectors.find((c) => c.id === id) ?? null) : null;
	};
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
		if (next.type === 'folder') {
			folderName = next.folder?.name ?? '';
			folderConnectorId = next.folder?.connector_id ?? '';
		}
		if (next.type === 'move') moveTarget = '';
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
				? updateFolder(target.folder.id, folderName, folderConnectorId || null)
				: createFolder(target.parent, folderName, folderConnectorId || null),
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

	function remove() {
		if (open?.type !== 'delete') return;
		const { kind, id } = open;
		run(kind === 'folder' ? deleteFolder(id) : deleteDevice(id), () => (selected = null));
	}

	/** What is dragged in the tree now (#214). */
	let dragged = $state<Dragged | null>(null);
	/** Something is dragged over the top level, which takes it. */
	let overTop = $state(false);
	/** The target chosen in *Move to …*: a folder's id, or `top`. */
	let moveTarget = $state('');

	/** Moves `item` into the folder `target`, or to the top level with null (#214). */
	function moveTo(item: Dragged, target: string | null) {
		run(
			item.kind === 'device' ? moveDevice(item.id, target ?? '') : moveFolder(item.id, target),
			() => {
				// Where it went, so that it is still in sight.
				if (target) setOpen(target, true);
			}
		);
	}

	function dropOn(target: string | null) {
		const item = dragged;
		dragged = null;
		overTop = false;
		if (item && tree && takes(tree, item, target)) moveTo(item, target);
	}

	const mover: Mover = {
		takes: (target) => !!dragged && !!tree && takes(tree, dragged, target),
		start: (item) => (dragged = item),
		end: () => {
			dragged = null;
			overTop = false;
		},
		drop: dropOn
	};

	function saveMove(event: SubmitEvent) {
		event.preventDefault();
		if (open?.type !== 'move' || !moveTarget) return;
		moveTo(open.item, moveTarget === 'top' ? null : moveTarget);
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
		if (target && tree && allows(target.role, 'connect')) tabs.open(target, tree.purpose_required);
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
		// Moving without a pointer (#214).
		const movable = kind === 'device' || kind === 'folder' ? kind : null;
		const move: MenuItem[] =
			change && movable
				? [
						{
							label: m.catalog_move(),
							icon: FolderInput,
							onselect: () => show({ type: 'move', item: { kind: movable, id }, name })
						}
					]
				: [];
		return [
			...(change ? [edit] : []),
			...move,
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
			case 'grants':
				return m.grants_title({ name: open.name });
			case 'delete':
				return m.catalog_delete();
			case 'request':
				return m.request_title({ name: open.name });
			case 'move':
				return m.catalog_move_title({ name: open.name });
			default:
				return '';
		}
	});

	const button =
		'inline-flex h-10 items-center gap-2 rounded-xl border border-line-strong bg-surface px-3.5 text-sm hover:bg-surface-2';
	const card = 'flex flex-col gap-3 rounded-card border border-line bg-surface p-5';
	// The property grid of a device.
	const groupHead =
		'bg-surface-2 px-4 py-1.5 text-xs font-semibold tracking-wide text-ink-2 uppercase';
	const grid = 'grid grid-cols-[minmax(8rem,14rem)_minmax(0,1fr)]';
	const key = 'border-t border-line px-4 py-2.5 text-ink-2';
	const value = 'border-t border-line px-4 py-2.5 break-words';
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
			class="flex w-full min-w-0 items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-sm hover:bg-surface-2 data-[active=true]:bg-surface-2 data-[active=true]:ring-1 data-[active=true]:ring-line-strong"
			data-active={highlighted || isSelected}
			aria-current={isSelected ? 'true' : undefined}
			onclick={() => choose(hit.kind, hit.id)}
			ondblclick={() => hit.kind === 'device' && connectTo(hit.id)}
		>
			{#if hit.protocol}
				<ProtocolChip protocol={hit.protocol} />
			{:else}
				<FolderClosed size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
			{/if}
			<!-- The name gets the whole line; host and path go small below it (#209). -->
			<span class="flex min-w-0 flex-col">
				<span class="truncate text-ink">{hit.name}</span>
				{#if hit.detail || hit.where}
					<span class="truncate text-[11px] leading-tight text-ink-3">
						{#if hit.detail}<span class="font-mono">{hit.detail}</span>{/if}
						{#if hit.detail && hit.where}
							·
						{/if}
						{hit.where}
					</span>
				{/if}
			</span>
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

{#snippet areaTabs()}
	<nav
		aria-label={m.devices_area()}
		class="grid grid-cols-2 gap-1 rounded-xl bg-surface-2 p-1 text-sm"
	>
		{#each [{ key: 'connections', label: m.devices_connections() }, { key: 'profiles', label: m.profiles_title() }] as tab (tab.key)}
			<button
				type="button"
				class="h-8 rounded-lg px-2 text-ink-2 aria-[current=page]:bg-surface aria-[current=page]:font-semibold aria-[current=page]:text-ink aria-[current=page]:shadow-sm"
				aria-current={area === tab.key ? 'page' : undefined}
				onclick={() => (area = tab.key as typeof area)}
			>
				{tab.label}
			</button>
		{/each}
	</nav>
{/snippet}

<div
	class="flex min-h-0 flex-1 flex-col overflow-y-auto lg:flex-row lg:overflow-hidden"
	class:hidden={tabs.active !== null}
>
	{#if area === 'profiles' && tree}
		<ProfilesView
			{tree}
			bind:selected={profileId}
			tabs={areaTabs}
			onchange={load}
			onshowdevice={showDevice}
		/>
	{:else}
		<aside
			class="flex shrink-0 flex-col gap-4 border-b border-line bg-sunken px-3 py-5 lg:w-84 lg:overflow-y-auto lg:border-r lg:border-b-0"
		>
			{@render areaTabs()}
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
				<nav aria-label={m.devices_title()} class="flex flex-col gap-1">
					{#if dragged && tree && takes(tree, dragged, null)}
						<!-- Only while a folder that may go to the top level is dragged (#214). -->
						<div
							role="presentation"
							class="flex items-center gap-2 rounded-lg border border-dashed border-line-strong px-3 py-2 text-sm text-ink-2"
							class:border-accent={overTop}
							class:bg-surface-2={overTop}
							ondragover={(event) => {
								event.preventDefault();
								overTop = true;
							}}
							ondragleave={() => (overTop = false)}
							ondrop={(event) => {
								event.preventDefault();
								dropOn(null);
							}}
						>
							<FolderInput size={15} class="shrink-0" aria-hidden="true" />
							{m.tree_drop_top_level()}
						</div>
					{/if}
					<ul role="tree" aria-label={m.devices_title()} class="flex flex-col gap-0.5">
						{#each roots as node (node.folder.id)}
							<FolderNodeView
								{node}
								{selected}
								expanded={(id) => openFolders.has(id)}
								onselect={choose}
								ontoggle={toggle}
								onopen={connectTo}
								{mover}
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
						{#if passedOn(folder.id) && allows(folder.role, 'connect')}
							<AskCustomer kind="folder" id={folder.id} />
						{/if}
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
							{#if device.reached_through}
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
								{#if allows(device.role, 'connect')}
									<DeviceAccess deviceId={device.id} />
									<AskCustomer kind="device" id={device.id} />
								{/if}
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
									onclick={() => connectTo(device.id)}
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

				<!-- Its properties, grouped as a connection manager lists them (#192). -->
				<div class="shrink-0 overflow-hidden rounded-card border border-line bg-surface text-sm">
					<h3 class={groupHead}>{m.device_group_connection()}</h3>
					<dl class={grid}>
						<dt class={key}>{m.field_protocol()}</dt>
						<dd class={value}>{PROTOCOL_LABELS[device.protocol]()}</dd>
						<dt class={key}>{m.field_host()}</dt>
						<dd class="{value} font-mono">{device.host}</dd>
						<dt class={key}>{m.field_port()}</dt>
						<dd class="{value} font-mono">{device.port}</dd>
						{#if device.protocol === 'rdp'}
							<dt class={key}>{m.field_keyboard_layout()}</dt>
							<dd class={value}>
								{device.keyboard_layout
									? keyboardLayoutLabel(device.keyboard_layout, getLocale())
									: m.keyboard_layout_default()}
							</dd>
						{/if}
					</dl>
					<h3 class={groupHead}>{m.device_group_sign_in()}</h3>
					<dl class={grid}>
						<dt class={key}>{m.field_auth_mode()}</dt>
						<dd class={value}>{AUTH_MODE_LABELS[device.auth_mode]()}</dd>
						{#if device.profile_id}
							{@const profile = tree?.profiles.find((p) => p.id === device.profile_id)}
							<dt class={key}>{m.field_profile()}</dt>
							<dd class={value}>
								{#if profile}
									<button
										type="button"
										class="text-left text-accent hover:underline"
										onclick={() => showProfile(profile.id)}
									>
										{profile.name} ·
										<span class="font-mono">
											{profile.domain ? `${profile.domain}\\${profile.username}` : profile.username}
										</span>
									</button>
								{:else}
									{m.profile_not_visible()}
								{/if}
							</dd>
						{:else if device.auth_mode === 'device'}
							<dt class={key}>{m.field_username()}</dt>
							<dd class="{value} font-mono">
								{device.domain ? `${device.domain}\\${device.username}` : device.username}
							</dd>
						{/if}
						{#if device.profile_id || device.auth_mode === 'device'}
							<dt class={key}>
								{device.secret_kind === 'ssh_key' && device.auth_mode === 'device'
									? m.credential_key()
									: m.field_password()}
							</dt>
							<dd class="{value} flex flex-col gap-2">
								<span class="flex items-center gap-2 text-ink-2">
									<Lock size={14} class="text-ok" aria-hidden="true" />
									{m.device_secret_on_server()}
								</span>
								{#if device.auth_mode === 'device' && device.key_fingerprint}
									<span class="font-mono text-xs break-all text-ink-2">
										{device.key_algorithm} · {device.key_fingerprint}
									</span>
								{/if}
								{#if device.auth_mode === 'device' && allows(device.role, 'reveal')}
									{#key device.id}
										<RevealSecret owner="devices" id={device.id} />
									{/key}
								{/if}
							</dd>
						{/if}
					</dl>
					{#if device.description}
						<h3 class={groupHead}>{m.device_group_general()}</h3>
						<dl class={grid}>
							<dt class={key}>{m.field_description()}</dt>
							<dd class="{value} whitespace-pre-line">{device.description}</dd>
						</dl>
					{/if}
				</div>

				<div class="grid gap-4 md:grid-cols-3">
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
					<section class="{card} md:col-span-2">
						<h3 class="eyebrow">{m.device_keywords()}</h3>
						<Keywords {device} onsaved={load} />
					</section>
					{#if allows(device.role, 'connect')}
						<section class="{card} md:col-span-3">
							<h3 class="eyebrow">{m.journal_title()}</h3>
							<Journal deviceId={device.id} />
						</section>
					{/if}
				</div>

				{@render requestedNotice(device.id)}
			{:else if tree}
				<p class="font-display text-2xl text-ink-3">{m.catalog_select_hint()}</p>
			{/if}
		</section>
	{/if}
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
					{#if connectors.length > 0 || folderConnectorId}
						{@const fromParent = passedOn(open.folder ? open.folder.parent_id : open.parent)}
						<label class="mt-3 block text-sm font-medium" for="folder-connector">
							{m.field_connector()}
						</label>
						<select
							id="folder-connector"
							class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
							bind:value={folderConnectorId}
						>
							<option value="">
								{m.connector_inherited({ name: fromParent?.name ?? m.connector_direct() })}
							</option>
							{#each connectors as connector (connector.id)}
								<option value={connector.id}>{connector.name}</option>
							{/each}
						</select>
						<p class="mt-1 text-xs text-ink-3">{m.folder_connector_hint()}</p>
					{/if}
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
					profiles={profilesWithin(tree, open.folderId)}
					{connectors}
					inherited={passedOn(open.folderId)}
					onsubmit={saveDevice}
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
			{:else if open?.type === 'move' && tree}
				{@const places = movePlaces(tree, open.item, getLocale())}
				{#if places.length === 0}
					<p class="text-sm">{m.catalog_move_none()}</p>
					<div class="mt-5 flex justify-end">
						<button
							type="button"
							class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
							onclick={() => (dialogOpen = false)}
						>
							{m.action_cancel()}
						</button>
					</div>
				{:else}
					<form onsubmit={saveMove}>
						<label class="block text-sm font-medium" for="move-target">
							{m.catalog_move_target()}
						</label>
						<select
							id="move-target"
							class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
							required
							bind:value={moveTarget}
						>
							<option value="" disabled>{m.catalog_move_choose()}</option>
							{#each places as place (place.id ?? 'top')}
								<option value={place.id ?? 'top'}
									>{place.id ? place.path : m.catalog_move_top()}</option
								>
							{/each}
						</select>
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
								{m.catalog_move_submit()}
							</button>
						</div>
					</form>
				{/if}
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
