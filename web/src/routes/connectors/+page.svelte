<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleHelp from '@lucide/svelte/icons/circle-help';
	import Download from '@lucide/svelte/icons/download';
	import Lock from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import Plus from '@lucide/svelte/icons/plus';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import {
		createConnector,
		deleteConnector,
		loadConnectors,
		type Connector,
		type CreatedConnector
	} from '$lib/api/connectors';
	import { errorMessage } from '$lib/api/errors';
	import Dialog from '$lib/components/Dialog.svelte';
	import ConnectorSetup from '$lib/connectors/ConnectorSetup.svelte';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';

	type Open =
		| { type: 'create' }
		| { type: 'created'; connector: CreatedConnector }
		| { type: 'download' }
		| { type: 'delete'; connector: Connector };

	let connectors = $state<Connector[]>([]);
	let error = $state<string | null>(null);
	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let name = $state('');
	let dialogError = $state<string | null>(null);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	/** A group or device the customer opened, with its end. */
	const openItem = (item: { name: string; until: string | null }) =>
		item.until
			? m.connector_open_item_until({ name: item.name, until: time.format(new Date(item.until)) })
			: m.connector_open_item({ name: item.name });

	async function load() {
		const result = await loadConnectors();
		if (result.ok) connectors = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		if (session.user?.admin) load();
	});

	function show(next: Open) {
		dialogError = null;
		name = '';
		open = next;
		dialogOpen = true;
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		const result = await createConnector(name);
		if (!result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		open = { type: 'created', connector: result.data };
		await load();
	}

	async function remove(connector: Connector) {
		const result = await deleteConnector(connector.id);
		if (!result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		dialogOpen = false;
		await load();
	}

	const title = $derived.by(() => {
		switch (open?.type) {
			case 'create':
				return m.connectors_new();
			case 'created':
				return m.connectors_token_title({ name: open.connector.name });
			case 'download':
				return m.connectors_download();
			case 'delete':
				return m.catalog_delete();
			default:
				return '';
		}
	});
</script>

<div class="flex flex-wrap items-start justify-between gap-4">
	<div>
		<h1 class="text-4xl font-semibold">{m.connectors_title()}</h1>
	</div>
	{#if session.user?.admin}
		<div class="flex flex-wrap gap-2">
			<!-- For connectors that exist: another host, a reinstall, an update (#186). -->
			<button
				type="button"
				class="inline-flex items-center gap-2 rounded-lg border border-line px-3 py-1.5 text-sm font-medium hover:bg-surface-2"
				onclick={() => show({ type: 'download' })}
			>
				<Download size={16} aria-hidden="true" />
				{m.connectors_download()}
			</button>
			<button
				type="button"
				class="inline-flex items-center gap-2 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
				onclick={() => show({ type: 'create' })}
			>
				<Plus size={16} aria-hidden="true" />
				{m.connectors_new()}
			</button>
		</div>
	{/if}
</div>

{#if !session.user?.admin}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else if error}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{:else if connectors.length === 0}
	<p class="mt-8 text-sm text-ink-2">{m.connectors_empty()}</p>
{:else}
	<div class="mt-6 overflow-x-auto rounded-card border border-line bg-surface">
		<table class="w-full text-left text-sm">
			<thead class="border-b border-line text-ink-2">
				<tr>
					<th class="px-4 py-2 font-medium">{m.field_name()}</th>
					<th class="px-4 py-2 font-medium">{m.connectors_col_state()}</th>
					<th class="px-4 py-2 font-medium">{m.connectors_col_access()}</th>
					<th class="px-4 py-2 font-medium">{m.connectors_col_streams()}</th>
					<th class="px-4 py-2 font-medium">{m.connectors_col_last_seen()}</th>
					<th class="px-4 py-2"><span class="sr-only">{m.catalog_delete()}</span></th>
				</tr>
			</thead>
			<tbody>
				{#each connectors as connector (connector.id)}
					<tr class="border-b border-line last:border-0">
						<td class="px-4 py-2">{connector.name}</td>
						<td class="px-4 py-2">
							<span class="inline-flex items-center gap-2">
								{#if connector.online}
									<span class="size-2 rounded-full bg-ok" aria-hidden="true"></span>
								{:else}
									<TriangleAlert size={13} class="text-warning" aria-hidden="true" />
								{/if}
								{connector.online ? m.connector_online() : m.connector_offline()}
							</span>
						</td>
						<td class="px-4 py-2">
							<span class="inline-flex items-center gap-2">
								{#if connector.access === 'open'}
									<LockOpen size={13} class="text-ok" aria-hidden="true" />
									{connector.open_until
										? m.connector_access_open_until({
												until: time.format(new Date(connector.open_until))
											})
										: m.connector_access_open()}
								{:else if connector.access === 'partly'}
									<LockOpen size={13} class="text-warning" aria-hidden="true" />
									{m.connector_access_partly()}
								{:else if connector.access === 'closed'}
									<Lock size={13} class="text-ink-2" aria-hidden="true" />
									{m.connector_access_closed()}
								{:else}
									<CircleHelp size={13} class="text-ink-3" aria-hidden="true" />
									<span class="text-ink-2">{m.connector_access_unknown()}</span>
								{/if}
							</span>
							{#if connector.open_groups.length > 0}
								<p class="mt-1 text-xs text-ink-2">
									{m.connector_open_groups()}
									{connector.open_groups.map(openItem).join(', ')}
								</p>
							{/if}
							{#if connector.open_devices.length > 0}
								<p class="mt-1 text-xs text-ink-2">
									{m.connector_open_devices()}
									{#each connector.open_devices as device, index (device.name)}
										<span title={`${device.address} : ${device.ports}`}
											>{openItem(device)}{index < connector.open_devices.length - 1
												? ', '
												: ''}</span
										>
									{/each}
								</p>
							{/if}
						</td>
						<td class="px-4 py-2 text-ink-2 tabular-nums">
							{m.connectors_streams({ open: connector.streams, total: connector.streams_carried })}
						</td>
						<td class="px-4 py-2 whitespace-nowrap text-ink-2 tabular-nums">
							{connector.last_seen_at
								? time.format(new Date(connector.last_seen_at))
								: m.connectors_never()}
						</td>
						<td class="px-4 py-2 text-right">
							<button
								type="button"
								class="rounded-md px-2 py-1 text-xs text-ink-2 hover:bg-surface-2 hover:text-ink"
								onclick={() => show({ type: 'delete', connector })}
							>
								{m.catalog_delete()}
							</button>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

<Dialog bind:open={dialogOpen} {title} wide={open?.type === 'created' || open?.type === 'download'}>
	{#if open?.type === 'create'}
		<form onsubmit={create}>
			<label class="block text-sm font-medium" for="connector-name">{m.field_name()}</label>
			<input
				id="connector-name"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				required
				maxlength="200"
				bind:value={name}
			/>
			{#if dialogError}
				<p class="mt-3 text-sm text-critical" role="alert">{dialogError}</p>
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
					{m.action_create()}
				</button>
			</div>
		</form>
	{:else if open?.type === 'created' || open?.type === 'download'}
		<ConnectorSetup
			token={open.type === 'created' ? open.connector.token : null}
			origin={window.location.origin}
		/>
		<div class="mt-5 flex justify-end">
			<button
				type="button"
				class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
				onclick={() => (dialogOpen = false)}
			>
				{m.action_close()}
			</button>
		</div>
	{:else if open?.type === 'delete'}
		{@const connector = open.connector}
		<p class="text-sm">{m.catalog_delete_confirm({ name: connector.name })}</p>
		{#if dialogError}
			<p class="mt-3 text-sm text-critical" role="alert">{dialogError}</p>
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
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={() => remove(connector)}
			>
				{m.catalog_delete()}
			</button>
		</div>
	{/if}
</Dialog>
