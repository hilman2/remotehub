<script lang="ts">
	import {
		DEFAULT_PORTS,
		authModesFor,
		KEYBOARD_LAYOUTS,
		PROTOCOLS,
		allows,
		type Credential,
		type CredentialKind,
		type Device,
		type DeviceInput,
		type KeyboardLayout,
		type Protocol
	} from '$lib/api/catalog';
	import type { Connector } from '$lib/api/connectors';
	import { getLocale } from '$lib/i18n';
	import { untrack } from 'svelte';
	import { m } from '$lib/paraglide/messages';
	import {
		AUTH_MODE_LABELS,
		CREDENTIAL_KIND_LABELS,
		PROTOCOL_LABELS,
		keyboardLayoutLabel
	} from './labels';
	import SecretFields, { secretInput } from './SecretFields.svelte';

	let {
		folderId,
		device = null,
		credentials,
		connectors = [],
		onsubmit,
		oncancel
	}: {
		folderId: string;
		device?: Device | null;
		credentials: Credential[];
		connectors?: Connector[];
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
	let keyboardLayout = $state<KeyboardLayout | ''>(start?.keyboard_layout ?? '');
	let connectorId = $state(start?.connector_id ?? '');
	let username = $state(start?.username ?? '');
	let domain = $state(start?.domain ?? '');
	let secretKind = $state<CredentialKind>(start?.secret_kind ?? 'password');
	let password = $state('');
	let privateKey = $state('');
	let passphrase = $state('');
	let certificate = $state('');

	// Keys are SSH's; the other protocols sign in with passwords.
	const kind = $derived<CredentialKind>(protocol === 'ssh' ? secretKind : 'password');
	// The server keeps the device's own secret for another target only when
	// the user may read it (#174); another kind, or a device without one,
	// needs it anew.
	const targetChanged = $derived(
		!!start &&
			(start.protocol !== protocol ||
				start.host !== host.trim() ||
				start.port !== Number(port) ||
				(start.connector_id ?? '') !== connectorId)
	);
	const retarget = $derived(
		!start ||
			start.auth_mode !== 'device' ||
			start.secret_kind !== kind ||
			(targetChanged && !allows(start.role, 'reveal'))
	);

	// Layouts by name in the UI's language; Unicode last.
	const layouts = $derived(
		KEYBOARD_LAYOUTS.map((layout) => ({
			layout,
			label: keyboardLayoutLabel(layout, getLocale())
		})).sort(
			(a, b) =>
				Number(a.layout === 'failsafe') - Number(b.layout === 'failsafe') ||
				a.label.localeCompare(b.label, getLocale())
		)
	);

	// Only credentials the user may use can be linked.
	const usable = $derived(credentials.filter((c) => allows(c.role, 'connect')));

	function changeProtocol(next: Protocol) {
		if (port === DEFAULT_PORTS[protocol]) port = DEFAULT_PORTS[next];
		protocol = next;
		if (!authModesFor(next).includes(authMode)) authMode = 'ask';
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
			description,
			keyboard_layout: protocol === 'rdp' ? keyboardLayout || null : null,
			connector_id: connectorId || null,
			...(authMode === 'device'
				? {
						username,
						domain,
						secret_kind: kind,
						...secretInput(kind, !retarget, { password, privateKey, passphrase, certificate })
					}
				: {})
		});
		password = passphrase = '';
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

	{#if connectors.length > 0 || connectorId}
		<label class={label} for="device-connector">{m.field_connector()}</label>
		<select id="device-connector" class={field} bind:value={connectorId}>
			<option value="">{m.connector_direct()}</option>
			{#each connectors as connector (connector.id)}
				<option value={connector.id}>{connector.name}</option>
			{/each}
			<!-- Kept as it is if the list did not load: saving must not move the device. -->
			{#if connectorId && !connectors.some((c) => c.id === connectorId)}
				<option value={connectorId}>{connectorId}</option>
			{/if}
		</select>
	{/if}

	{#if protocol === 'rdp'}
		<label class={label} for="device-keyboard">{m.field_keyboard_layout()}</label>
		<select id="device-keyboard" class={field} bind:value={keyboardLayout}>
			<option value="">{m.keyboard_layout_default()}</option>
			{#each layouts as { layout, label: text } (layout)}
				<option value={layout}>{text}</option>
			{/each}
		</select>
	{/if}

	<label class={label} for="device-auth">{m.field_auth_mode()}</label>
	<select id="device-auth" class={field} bind:value={authMode}>
		{#each authModesFor(protocol) as mode (mode)}
			<option value={mode} disabled={mode === 'stored' && usable.length === 0}>
				{AUTH_MODE_LABELS[mode]()}
			</option>
		{/each}
	</select>
	{#if authMode === 'laps'}
		<p class="mt-1 text-xs text-ink-3">{m.auth_laps_hint()}</p>
	{/if}
	{#if authMode === 'certificate'}
		<p class="mt-1 text-xs break-words text-ink-3">
			{m.auth_certificate_hint({ url: new URL('/api/ssh-ca.pub', window.location.href).href })}
		</p>
	{/if}

	{#if authMode === 'device'}
		<div class="grid grid-cols-2 gap-3">
			<div>
				<label class={label} for="device-username">
					{protocol === 'vnc' ? m.credentials_username_optional() : m.field_username()}
				</label>
				<input
					id="device-username"
					class={field}
					required={protocol !== 'vnc'}
					maxlength="256"
					autocomplete="off"
					spellcheck="false"
					bind:value={username}
				/>
			</div>
			<div>
				<label class={label} for="device-domain">{m.field_domain()}</label>
				<input
					id="device-domain"
					class={field}
					maxlength="256"
					autocomplete="off"
					spellcheck="false"
					bind:value={domain}
				/>
			</div>
		</div>
		{#if protocol === 'ssh'}
			<label class={label} for="device-secret-kind">{m.credential_kind()}</label>
			<select id="device-secret-kind" class={field} bind:value={secretKind}>
				{#each Object.entries(CREDENTIAL_KIND_LABELS) as [value, text] (value)}
					<option {value}>{text()}</option>
				{/each}
			</select>
		{/if}
		<SecretFields
			id="device"
			{kind}
			keep={!retarget}
			bind:password
			bind:privateKey
			bind:passphrase
			bind:certificate
		/>
		{#if retarget && start?.auth_mode === 'device'}
			<p class="mt-1 text-xs text-ink-3">{m.auth_device_target_changed()}</p>
		{/if}
	{/if}

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
