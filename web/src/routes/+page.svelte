<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Clock from '@lucide/svelte/icons/clock';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import MonitorSmartphone from '@lucide/svelte/icons/monitor-smartphone';
	import Plug from '@lucide/svelte/icons/plug';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
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
		KIND_LABELS,
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
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
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
		<p class="mt-3 flex items-center gap-2 text-sm" role="status">
			<CircleCheck size={16} class="text-ok" aria-hidden="true" />
			{m.request_sent()}
			<a class="underline" href={resolve('/requests')}>{m.nav_requests()}</a>
		</p>
	{/if}
{/snippet}

<div class="flex flex-wrap items-center gap-3">
	<h1 class="text-2xl font-semibold tracking-tight">{m.devices_title()}</h1>
	<label class="relative ml-auto w-full max-w-sm">
		<span class="sr-only">{m.catalog_search()}</span>
		<Search size={16} class="absolute top-2.5 left-3 text-ink-3" aria-hidden="true" />
		<input
			type="search"
			class="w-full rounded-lg border border-line bg-surface py-2 pr-3 pl-9 text-sm"
			placeholder={m.catalog_search()}
			bind:value={query}
		/>
	</label>
	{#if tree?.may_create_top_level}
		<button
			type="button"
			class={button}
			onclick={() => show({ type: 'folder', parent: null, folder: null })}
		>
			<FolderPlus size={16} aria-hidden="true" />
			{m.catalog_new_folder()}
		</button>
	{/if}
</div>

{#if error && !dialogOpen}
	<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{/if}

{#if tree && tree.folders.length === 0}
	<section
		class="mt-8 flex flex-col items-center gap-3 rounded-card border border-dashed border-line bg-surface px-6 py-16 text-center"
	>
		<MonitorSmartphone size={40} class="text-ink-3" aria-hidden="true" />
		<h2 class="text-lg font-medium">{m.devices_empty_title()}</h2>
		<p class="max-w-md text-sm text-ink-2">
			{tree.may_create_top_level ? m.catalog_empty_admin() : m.devices_empty_text()}
		</p>
	</section>
{:else if tree}
	<div class="mt-6 grid gap-6 lg:grid-cols-[minmax(18rem,26rem)_1fr]">
		<nav class="rounded-card border border-line bg-surface p-2" aria-label={m.devices_title()}>
			{#if roots.length === 0}
				<p class="p-3 text-sm text-ink-2">{m.catalog_no_match()}</p>
			{:else}
				<ul role="tree" aria-label={m.devices_title()}>
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

		<section class="rounded-card border border-line bg-surface p-6" aria-live="polite">
			{#if folder}
				<p class="text-xs text-ink-3">{KIND_LABELS.folder()}</p>
				<h2 class="text-xl font-semibold">{folder.name}</h2>
				<p class="mt-1 text-sm text-ink-2">
					{pathTo(tree, folder.parent_id)
						.map((f) => f.name)
						.join(' / ')}
				</p>
				{#if folder.role}
					<p class="mt-3 text-sm text-ink-2">
						{m.catalog_access({ role: ROLE_LABELS[folder.role]() })}
					</p>
				{:else}
					<p class="mt-3 text-sm text-ink-2">{m.catalog_path_only()}</p>
				{/if}
				<div class="mt-5 flex flex-wrap gap-2">
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
							class="{button} text-critical"
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
				<p class="text-xs text-ink-3">{KIND_LABELS.device()}</p>
				<h2 class="text-xl font-semibold">{device.name}</h2>
				<p class="mt-1 text-sm text-ink-2">
					{pathTo(tree, device.folder_id)
						.map((f) => f.name)
						.join(' / ')}
				</p>
				<dl class="mt-4 grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
					<dt class="text-ink-2">{m.field_protocol()}</dt>
					<dd>{PROTOCOL_LABELS[device.protocol]()}</dd>
					<dt class="text-ink-2">{m.field_host()}</dt>
					<dd class="font-mono">{device.host}:{device.port}</dd>
					{#if device.connector_id}
						{@const connector = connectorOf(device)}
						<dt class="text-ink-2">{m.field_connector()}</dt>
						<dd>
							{connector?.name ?? ''}
							<span class="text-ink-3">
								· {connector?.online ? m.connector_online() : m.connector_offline()}
							</span>
						</dd>
					{/if}
					<dt class="text-ink-2">{m.field_auth_mode()}</dt>
					<dd>
						{AUTH_MODE_LABELS[device.auth_mode]()}
						{#if device.credential_id}
							<span class="text-ink-2">
								· {tree.credentials.find((c) => c.id === device.credential_id)?.name ?? ''}
							</span>
						{/if}
					</dd>
					{#if device.protocol === 'rdp'}
						<dt class="text-ink-2">{m.field_keyboard_layout()}</dt>
						<dd>
							{device.keyboard_layout
								? keyboardLayoutLabel(device.keyboard_layout, getLocale())
								: m.keyboard_layout_default()}
						</dd>
					{/if}
					{#if device.protocol === 'rdp' || device.protocol === 'https'}
						<dt class="text-ink-2">{m.device_certificate()}</dt>
						<dd>
							{#if device.certificate_fingerprint}
								<span class="font-mono text-xs break-all">{device.certificate_fingerprint}</span>
								{#if allows(device.role, 'edit')}
									<button
										type="button"
										class="ml-2 text-xs text-ink-3 underline hover:text-ink"
										onclick={() => run(resetHostKey(device.id))}
									>
										{m.device_forget_host_key()}
									</button>
								{/if}
							{:else}
								<span class="text-ink-2">{m.device_host_key_pending()}</span>
							{/if}
						</dd>
					{/if}
					{#if device.protocol === 'ssh'}
						<dt class="text-ink-2">{m.device_host_key()}</dt>
						<dd>
							{#if device.host_key_fingerprint}
								<span class="font-mono text-xs break-all">{device.host_key_fingerprint}</span>
								{#if allows(device.role, 'edit')}
									<button
										type="button"
										class="ml-2 text-xs text-ink-3 underline hover:text-ink"
										onclick={() => run(resetHostKey(device.id))}
									>
										{m.device_forget_host_key()}
									</button>
								{/if}
							{:else}
								<span class="text-ink-2">{m.device_host_key_pending()}</span>
							{/if}
						</dd>
					{/if}
					{#if device.description}
						<dt class="text-ink-2">{m.field_description()}</dt>
						<dd class="whitespace-pre-line">{device.description}</dd>
					{/if}
				</dl>
				<p class="mt-3 text-sm text-ink-2">
					{m.catalog_access({ role: ROLE_LABELS[device.role]() })}
				</p>
				<div class="mt-5 flex flex-wrap gap-2">
					{#if allows(device.role, 'connect')}
						<!-- A browser tab per session: several sessions side by side for free. -->
						<a
							href={resolve('/connect/[id]', { id: device.id })}
							target="_blank"
							rel="noopener"
							class="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
						>
							<Plug size={16} aria-hidden="true" />
							{m.device_connect()}
						</a>
					{/if}
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
							class="{button} text-critical"
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
				<p class="text-xs text-ink-3">
					{KIND_LABELS.credential()} · {CREDENTIAL_KIND_LABELS[credential.kind]()}
				</p>
				<h2 class="text-xl font-semibold">{credential.name}</h2>
				<p class="mt-1 text-sm text-ink-2">
					{pathTo(tree, credential.folder_id)
						.map((f) => f.name)
						.join(' / ')}
				</p>
				<dl class="mt-4 grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
					<dt class="text-ink-2">{m.field_username()}</dt>
					<dd class="font-mono">{credential.username}</dd>
					{#if credential.domain}
						<dt class="text-ink-2">{m.field_domain()}</dt>
						<dd class="font-mono">{credential.domain}</dd>
					{/if}
					{#if credential.kind === 'ssh_key'}
						<dt class="text-ink-2">{m.credential_key()}</dt>
						<dd>
							<span class="font-mono text-xs">{credential.key_algorithm}</span>
							<span class="block font-mono text-xs break-all text-ink-2">
								{credential.key_fingerprint}
							</span>
							<span class="mt-1 block text-ink-2">{m.credential_key_hidden()}</span>
						</dd>
						<dt class="text-ink-2">{m.credential_certificate()}</dt>
						<dd>
							{credential.has_certificate
								? m.credential_certificate_yes()
								: m.credential_certificate_no()}
						</dd>
					{:else}
						<dt class="text-ink-2">{m.field_password()}</dt>
						<dd class="text-ink-2">{m.credential_hidden()}</dd>
					{/if}
				</dl>
				<p class="mt-3 text-sm text-ink-2">
					{m.catalog_access({ role: ROLE_LABELS[credential.role]() })} · {m.credential_version({
						version: credential.version
					})}
				</p>
				<div class="mt-5 flex flex-wrap gap-2">
					{#if allows(credential.role, 'edit')}
						<button
							type="button"
							class={button}
							onclick={() =>
								show({ type: 'credential', folderId: credential.folder_id, credential })}
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
							class="{button} text-critical"
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
			{:else}
				<p class="text-sm text-ink-2">{m.catalog_select_hint()}</p>
			{/if}
		</section>
	</div>
{/if}

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
