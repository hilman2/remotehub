<script lang="ts">
	import {
		AUTH_MODES,
		DEFAULT_PORTS,
		PROTOCOLS,
		allows,
		type Credential,
		type Device,
		type DeviceInput,
		type Protocol
	} from '$lib/api/catalog';
	import { untrack } from 'svelte';
	import { m } from '$lib/paraglide/messages';
	import { AUTH_MODE_LABELS, PROTOCOL_LABELS } from './labels';

	let {
		folderId,
		device = null,
		credentials,
		onsubmit,
		oncancel
	}: {
		folderId: string;
		device?: Device | null;
		credentials: Credential[];
		onsubmit: (input: DeviceInput) => void;
		oncancel: () => void;
	} = $props();

	// Initial values; the form owns them afterwards.
	const start = untrack(() => $state.snapshot(device));
	let name = $state(start?.name ?? '');
	let protocol = $state<Protocol>(start?.protocol ?? 'ssh');
	let host = $state(start?.host ?? '');
	let port = $state(start?.port ?? DEFAULT_PORTS.ssh);
	let authMode = $state(start?.auth_mode ?? 'ask');
	let credentialId = $state(start?.credential_id ?? '');
	let description = $state(start?.description ?? '');

	// Only credentials the user may use can be linked.
	const usable = $derived(credentials.filter((c) => allows(c.role, 'connect')));

	function changeProtocol(next: Protocol) {
		if (port === DEFAULT_PORTS[protocol]) port = DEFAULT_PORTS[next];
		protocol = next;
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		onsubmit({
			folder_id: start?.folder_id ?? folderId,
			name,
			protocol,
			host,
			port: Number(port),
			auth_mode: authMode,
			credential_id: authMode === 'stored' ? credentialId || null : null,
			description
		});
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit}>
	<label class="block text-sm font-medium" for="device-name">{m.field_name()}</label>
	<input id="device-name" class={field} required maxlength="200" bind:value={name} />

	<div class="grid grid-cols-[1fr_7rem] gap-3">
		<div>
			<label class={label} for="device-protocol">{m.field_protocol()}</label>
			<select
				id="device-protocol"
				class={field}
				value={protocol}
				onchange={(e) => changeProtocol(e.currentTarget.value as Protocol)}
			>
				{#each PROTOCOLS as p (p)}
					<option value={p}>{PROTOCOL_LABELS[p]()}</option>
				{/each}
			</select>
		</div>
		<div>
			<label class={label} for="device-port">{m.field_port()}</label>
			<input
				id="device-port"
				class={field}
				type="number"
				min="1"
				max="65535"
				required
				bind:value={port}
			/>
		</div>
	</div>

	<label class={label} for="device-host">{m.field_host()}</label>
	<input
		id="device-host"
		class="{field} font-mono"
		required
		maxlength="253"
		autocomplete="off"
		spellcheck="false"
		bind:value={host}
	/>

	<label class={label} for="device-auth">{m.field_auth_mode()}</label>
	<select id="device-auth" class={field} bind:value={authMode}>
		{#each AUTH_MODES as mode (mode)}
			<option value={mode} disabled={mode === 'stored' && usable.length === 0}>
				{AUTH_MODE_LABELS[mode]()}
			</option>
		{/each}
	</select>

	{#if authMode === 'stored'}
		<label class={label} for="device-credential">{m.field_credential()}</label>
		<select id="device-credential" class={field} required bind:value={credentialId}>
			{#each usable as credential (credential.id)}
				<option value={credential.id}>{credential.name} · {credential.username}</option>
			{/each}
		</select>
	{/if}

	<label class={label} for="device-description">{m.field_description()}</label>
	<textarea id="device-description" class={field} rows="2" maxlength="2000" bind:value={description}
	></textarea>

	<div class="mt-5 flex justify-end gap-2">
		<button
			type="button"
			class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
			onclick={oncancel}
		>
			{m.action_cancel()}
		</button>
		<button
			type="submit"
			class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
		>
			{device ? m.action_save() : m.action_create()}
		</button>
	</div>
</form>
