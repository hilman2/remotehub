<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';
	import ShieldAlert from '@lucide/svelte/icons/shield-alert';
	import { page } from '$app/state';
	import { allows, isGraphical, loadTree, resetHostKey, type Device } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import ProtocolChip from '$lib/catalog/ProtocolChip.svelte';
	import DisplayView from '$lib/display/DisplayView.svelte';
	import { failureOf, type Failure } from '$lib/display/tunnel';
	import { m } from '$lib/paraglide/messages';
	import TerminalView from '$lib/terminal/TerminalView.svelte';
	import type { ServerEvent as DisplayEvent } from '$lib/display/tunnel';
	import type { Credentials, ServerEvent } from '$lib/terminal/connection';

	type Status =
		| { kind: 'connecting' }
		/** `pinned`: SSH host key or RDP certificate, pinned by this connection. */
		| { kind: 'connected'; fingerprint: string | null; certificate: boolean; pinned: boolean }
		| { kind: 'closed'; exitStatus: number | null }
		| { kind: 'lost' }
		| { kind: 'error'; code: string; params: Record<string, unknown> }
		/** guacd ended an RDP or VNC session. */
		| { kind: 'failed'; failure: Failure; detail: string };

	const FAILURES: Record<Failure, () => string> = {
		target: m.display_failed_target,
		auth: m.display_failed_auth,
		ended: m.display_ended,
		failed: m.display_failed
	};

	let device = $state<Device | null>(null);
	let error = $state<string | null>(null);
	let credentials = $state<Credentials | null>(null);
	let username = $state('');
	let password = $state('');
	let status = $state<Status>({ kind: 'connecting' });
	// Remounting the view starts a new connection.
	let attempt = $state(0);

	const graphical = $derived(device !== null && isGraphical(device.protocol));
	const needsCredentials = $derived(device?.auth_mode === 'ask' && credentials === null);
	const running = $derived(device !== null && !needsCredentials);

	$effect(() => {
		const id = page.params.id;
		loadTree().then((result) => {
			if (!result.ok) {
				error = errorMessage(result.code);
				return;
			}
			device = result.data.devices.find((d) => d.id === id) ?? null;
			if (!device) error = errorMessage('not_found');
		});
	});

	$effect(() => {
		if (device) document.title = `${device.name} · remotehub`;
	});

	function onevent(event: ServerEvent | DisplayEvent) {
		if (event.type === 'connected') {
			status =
				'host_key_fingerprint' in event
					? {
							kind: 'connected',
							fingerprint: event.host_key_fingerprint,
							certificate: false,
							pinned: event.pinned
						}
					: {
							kind: 'connected',
							fingerprint: event.certificate_fingerprint,
							certificate: true,
							pinned: event.pinned
						};
		} else if (event.type === 'closed') {
			status = { kind: 'closed', exitStatus: event.exit_status };
		} else {
			status = { kind: 'error', code: event.code, params: event.params };
		}
	}

	function onfailure(code: number, detail: string) {
		status = { kind: 'failed', failure: failureOf(code), detail };
	}

	function onend() {
		if (status.kind === 'connecting' || status.kind === 'connected') status = { kind: 'lost' };
	}

	function reconnect() {
		status = { kind: 'connecting' };
		// Asked credentials are asked again.
		if (device?.auth_mode === 'ask') credentials = null;
		attempt += 1;
	}

	async function trustNewKey() {
		if (!device) return;
		const result = await resetHostKey(device.id);
		if (result.ok) reconnect();
		else error = errorMessage(result.code);
	}

	function signIn(event: SubmitEvent) {
		event.preventDefault();
		credentials = { username, password };
		password = '';
		status = { kind: 'connecting' };
	}

	const param = (name: string) => String((status.kind === 'error' && status.params[name]) || '');
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
</script>

{#if error}
	<p class="flex items-center gap-2 px-6 py-10 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{:else if device && needsCredentials}
	<form
		class="mx-auto mt-16 w-full max-w-sm rounded-card border border-line bg-surface p-7"
		onsubmit={signIn}
		autocomplete="off"
	>
		<h1 class="text-2xl font-semibold">{m.terminal_credentials_title({ name: device.name })}</h1>
		<p class="mt-1 text-sm text-ink-2">{m.terminal_credentials_hint()}</p>
		<!-- VNC servers mostly know only a password. -->
		<label class="mt-4 block text-sm font-medium" for="target-username">
			{device.protocol === 'vnc' ? m.credentials_username_optional() : m.field_username()}
		</label>
		<input
			id="target-username"
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
			required={device.protocol !== 'vnc'}
			spellcheck="false"
			bind:value={username}
		/>
		<label class="mt-3 block text-sm font-medium" for="target-password">{m.field_password()}</label>
		<input
			id="target-password"
			type="password"
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
			required
			bind:value={password}
		/>
		<button
			type="submit"
			class="mt-6 h-12 w-full rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110"
		>
			{m.terminal_connect()}
		</button>
	</form>
{:else if device}
	<div class="flex min-h-80 flex-1 flex-col">
		<div
			class="flex min-h-12 flex-wrap items-center gap-3 border-b border-line bg-sunken px-4 py-2 text-sm sm:px-6"
			role="status"
		>
			<ProtocolChip protocol={device.protocol} />
			<h1 class="text-base font-semibold">{device.name}</h1>
			<span class="font-mono text-xs text-ink-3">{device.host}:{device.port}</span>
			<span class="ml-auto inline-flex items-center gap-1.5 text-ink-2">
				{#if status.kind === 'connecting'}
					<LoaderCircle size={15} class="animate-spin" aria-hidden="true" />
					{m.terminal_connecting({ name: device.name })}
				{:else if status.kind === 'connected'}
					<CircleCheck size={15} class="text-ok" aria-hidden="true" />
					{#if status.fingerprint === null}
						{m.display_connected()}
					{:else if status.certificate}
						{status.pinned
							? m.display_certificate_pinned({ fingerprint: status.fingerprint })
							: m.display_connected_certificate({ fingerprint: status.fingerprint })}
					{:else if status.pinned}
						{m.terminal_pinned({ fingerprint: status.fingerprint })}
					{:else}
						{m.terminal_connected({ fingerprint: status.fingerprint })}
					{/if}
				{:else if status.kind === 'closed'}
					{status.exitStatus === null
						? m.terminal_closed()
						: m.terminal_closed_status({ status: status.exitStatus })}
				{:else if status.kind === 'lost'}
					<CircleAlert size={15} class="text-critical" aria-hidden="true" />
					{m.terminal_lost()}
				{:else if status.kind === 'failed'}
					<CircleAlert size={15} class="text-critical" aria-hidden="true" />
					{FAILURES[status.failure]()}
				{:else}
					<CircleAlert size={15} class="text-critical" aria-hidden="true" />
					{errorMessage(status.code)}
				{/if}
			</span>
			{#if status.kind !== 'connecting' && status.kind !== 'connected'}
				<button type="button" class={button} onclick={reconnect}>
					<RotateCw size={15} aria-hidden="true" />
					{m.terminal_reconnect()}
				</button>
			{/if}
		</div>

		{#if status.kind === 'failed' && status.detail}
			<!-- guacd's own words, for the administrator. -->
			<p class="px-4 pt-3 text-xs text-ink-3 sm:px-6">
				{m.display_detail({ detail: status.detail })}
			</p>
		{/if}

		{#if status.kind === 'error' && (status.code === 'host_key_changed' || status.code === 'certificate_changed')}
			<div
				class="m-4 rounded-card border border-critical/50 bg-surface p-5 text-sm sm:mx-6"
				role="alert"
			>
				<p class="flex items-start gap-2">
					<ShieldAlert size={18} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
					<span>
						{(status.code === 'certificate_changed'
							? m.display_certificate_changed
							: m.terminal_host_key_changed)({
							name: device.name,
							expected: param('expected'),
							presented: param('presented')
						})}
					</span>
				</p>
				{#if allows(device.role, 'edit')}
					<button type="button" class="{button} mt-3" onclick={trustNewKey}>
						{status.code === 'certificate_changed'
							? m.display_trust_new_certificate()
							: m.terminal_trust_new_key()}
					</button>
				{/if}
			</div>
		{/if}

		{#if running}
			<div class="min-h-0 flex-1">
				{#key attempt}
					{#if graphical}
						<DisplayView
							deviceId={device.id}
							name={device.name}
							{credentials}
							{onevent}
							{onfailure}
							{onend}
						/>
					{:else}
						<TerminalView deviceId={device.id} {credentials} {onevent} {onend} />
					{/if}
				{/key}
			</div>
		{/if}
	</div>
{/if}
