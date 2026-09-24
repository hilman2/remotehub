<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Clock from '@lucide/svelte/icons/clock';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Lock from '@lucide/svelte/icons/lock';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
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
	import { filter, nest, pathTo } from '$lib/catalog/tree';
	import Dialog from '$lib/components/Dialog.svelte';
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
	const collapsed = new SvelteSet<string>();
	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let error = $state<string | null>(null);
	let folderName = $state('');
	/** The object whose access request was just sent, for the notice. */
	let requested = $state<string | null>(null);

	const roots = $derived(tree ? filter(nest(tree, getLocale()), query) : []);
	const searching = $derived(query.trim().length > 0);
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
		const [result, sites] = await Promise.all([loadTree(), loadConnectors()]);
		if (result.ok) tree = result.data;
		else error = errorMessage(result.code);
		if (sites.ok) connectors = sites.data;
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

	const created = (kind: ObjectKind) => (data: unknown) => {
		const id = (data as { id?: string } | undefined)?.id;
		if (id) selected = { kind, id };
	};

	function saveFolder(event: SubmitEvent) {
		event.preventDefault();
		if (open?.type !== 'folder') return;
		const target = open;
		run(
			target.folder
				? renameFolder(target.folder.id, folderName)
				: createFolder(target.parent, folderName),
			target.folder ? undefined : created('folder')
		);
	}

	function saveDevice(input: DeviceInput) {
		if (open?.type !== 'device') return;
		const target = open;
		run(
			target.device ? updateDevice(target.device.id, input) : createDevice(input),
			target.device ? undefined : created('device')
		);
	}

	function saveCredential(input: CredentialInput) {
		if (open?.type !== 'credential') return;
		const target = open;
		run(
			target.credential ? updateCredential(target.credential.id, input) : createCredential(input),
			target.credential ? undefined : created('credential')
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

	function toggle(id: string) {
		if (collapsed.has(id)) collapsed.delete(id);
		else collapsed.add(id);
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
	const danger =
		'inline-flex h-10 items-center gap-2 rounded-xl px-3.5 text-sm text-critical hover:bg-surface-2';
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

<div class="flex min-h-0 flex-1 flex-col overflow-y-auto lg:flex-row lg:overflow-hidden">
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
			/>
		</label>
		{#if tree && tree.folders.length > 0}
			<nav aria-label={m.devices_title()}>
				{#if roots.length === 0}
					<p class="px-2 text-sm text-ink-2">{m.catalog_no_match()}</p>
				{:else}
					<ul role="tree" aria-label={m.devices_title()} class="flex flex-col gap-0.5">
						{#each roots as node (node.folder.id)}
							<FolderNodeView
								{node}
								{selected}
								expanded={(id) => searching || !collapsed.has(id)}
								onselect={(kind, id) => (selected = { kind, id })}
								ontoggle={toggle}
							/>
						{/each}
					</ul>
				{/if}
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
			{@render heading(path(folder.parent_id), folder.name)}
			{#if folder.role}
				<div class="flex flex-wrap gap-2">
					<span class="chip">{m.catalog_access({ role: ROLE_LABELS[folder.role]() })}</span>
				</div>
			{/if}
			<div class="flex flex-wrap gap-2">
				{#if allows(folder.role, 'edit')}
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
				{/if}
				{#if allows(folder.role, 'manage')}
					<button
						type="button"
						class={button}
						onclick={() => show({ type: 'folder', parent: folder.id, folder: null })}
					>
						<FolderPlus size={16} aria-hidden="true" />
						{m.catalog_new_subfolder()}
					</button>
					<button
						type="button"
						class={button}
						onclick={() => show({ type: 'folder', parent: folder.parent_id, folder })}
					>
						{m.catalog_rename()}
					</button>
					<button
						type="button"
						class={button}
						onclick={() =>
							show({ type: 'grants', kind: 'folder', id: folder.id, name: folder.name })}
					>
						<ShieldCheck size={16} aria-hidden="true" />
						{m.catalog_permissions()}
					</button>
					<button
						type="button"
						class={danger}
						onclick={() =>
							show({ type: 'delete', kind: 'folder', id: folder.id, name: folder.name })}
					>
						{m.catalog_delete()}
					</button>
				{/if}
				{@render requestAccess('folder', folder.id, folder.name, folder.role)}
			</div>
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
				{#if allows(device.role, 'connect')}
					<!-- A browser tab per session: several sessions side by side for free. -->
					<a
						href={resolve('/connect/[id]', { id: device.id })}
						target="_blank"
						rel="noopener"
						class="inline-flex h-14 items-center rounded-2xl bg-accent px-8 font-display text-lg font-semibold text-accent-ink hover:brightness-110"
					>
						{m.device_connect()}
					</a>
				{/if}
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

			<div class="flex flex-wrap gap-2">
				{#if allows(device.role, 'edit')}
					<button
						type="button"
						class={button}
						onclick={() => show({ type: 'device', folderId: device.folder_id, device })}
					>
						{m.catalog_edit()}
					</button>
				{/if}
				{#if allows(device.role, 'manage')}
					<button
						type="button"
						class={button}
						onclick={() =>
							show({ type: 'grants', kind: 'device', id: device.id, name: device.name })}
					>
						<ShieldCheck size={16} aria-hidden="true" />
						{m.catalog_permissions()}
					</button>
				{/if}
				{#if allows(device.role, 'edit')}
					<button
						type="button"
						class={danger}
						onclick={() =>
							show({ type: 'delete', kind: 'device', id: device.id, name: device.name })}
					>
						{m.catalog_delete()}
					</button>
				{/if}
				{@render requestAccess('device', device.id, device.name, device.role)}
			</div>
			{@render requestedNotice(device.id)}
		{:else if credential}
			{@render heading(path(credential.folder_id), credential.name)}
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
			<div class="flex flex-wrap gap-2">
				{#if allows(credential.role, 'edit')}
					<button
						type="button"
						class={button}
						onclick={() => show({ type: 'credential', folderId: credential.folder_id, credential })}
					>
						{m.catalog_edit()}
					</button>
				{/if}
				{#if allows(credential.role, 'manage')}
					<button
						type="button"
						class={button}
						onclick={() =>
							show({
								type: 'grants',
								kind: 'credential',
								id: credential.id,
								name: credential.name
							})}
					>
						<ShieldCheck size={16} aria-hidden="true" />
						{m.catalog_permissions()}
					</button>
				{/if}
				{#if allows(credential.role, 'edit')}
					<button
						type="button"
						class={danger}
						onclick={() =>
							show({
								type: 'delete',
								kind: 'credential',
								id: credential.id,
								name: credential.name
							})}
					>
						{m.catalog_delete()}
					</button>
				{/if}
				{@render requestAccess('credential', credential.id, credential.name, credential.role)}
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
