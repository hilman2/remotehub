<script lang="ts">
	/**
	 * One session with a device: asked credentials if needed, then the
	 * terminal or the remote display, edge to edge. How far it got stands in
	 * the footer; an ended session says why over its picture. Used as a tab
	 * inside remotehub (#85) and alone in its own window (/connect/[id]).
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleMinus from '@lucide/svelte/icons/circle-minus';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';
	import ShieldAlert from '@lucide/svelte/icons/shield-alert';
	import { allows, isGraphical, resetHostKey, type Device } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import DisplayView from '$lib/display/DisplayView.svelte';
	import { failureOf, type Failure, type ServerEvent as DisplayEvent } from '$lib/display/tunnel';
	import { m } from '$lib/paraglide/messages';
	import TerminalView from '$lib/terminal/TerminalView.svelte';
	import type { Credentials, ServerEvent } from '$lib/terminal/connection';
	import { shown } from './status.svelte';
	import type { Phase } from './tabs.svelte';

	let {
		device,
		visible = true,
		onphase
	}: {
		device: Device;
		/** Hidden tabs keep running, but do not follow the window's size. */
		visible?: boolean;
		/** For the tab: how far the session got. */
		onphase?: (phase: Phase) => void;
	} = $props();

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

	let error = $state<string | null>(null);
	let credentials = $state<Credentials | null>(null);
	let username = $state('');
	let password = $state('');
	let status = $state<Status>({ kind: 'connecting' });
	// Remounting the view starts a new connection.
	let attempt = $state(0);

	const graphical = $derived(isGraphical(device.protocol));
	const needsCredentials = $derived(device.auth_mode === 'ask' && credentials === null);
	const phase = $derived<Phase>(
		status.kind === 'connecting' || status.kind === 'connected' || status.kind === 'closed'
			? status.kind
			: 'failed'
	);
	/** The device presented another host key or certificate than the pinned one. */
	const changed = $derived(
		status.kind === 'error' &&
			(status.code === 'host_key_changed' || status.code === 'certificate_changed')
	);

	const text = $derived.by(() => {
		switch (status.kind) {
			case 'connecting':
				return m.terminal_connecting({ name: device.name });
			case 'connected':
				if (status.fingerprint === null) return m.display_connected();
				if (status.certificate) {
					return status.pinned
						? m.display_certificate_pinned({ fingerprint: status.fingerprint })
						: m.display_connected_certificate({ fingerprint: status.fingerprint });
				}
				return status.pinned
					? m.terminal_pinned({ fingerprint: status.fingerprint })
					: m.terminal_connected({ fingerprint: status.fingerprint });
			case 'closed':
				return status.exitStatus === null
					? m.terminal_closed()
					: m.terminal_closed_status({ status: status.exitStatus });
			case 'lost':
				return m.terminal_lost();
			case 'failed':
				return FAILURES[status.failure]();
			case 'error':
				return errorMessage(status.code);
		}
	});

	$effect(() => {
		onphase?.(phase);
	});

	// The footer tells about this session while it is the one on screen.
	const owner = shown.claim();
	$effect(() => {
		if (visible && !error && !needsCredentials) shown.set(owner, { phase, text });
		else shown.release(owner);
	});
	$effect(() => () => shown.release(owner));

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
		if (device.auth_mode === 'ask') credentials = null;
		attempt += 1;
	}

	async function trustNewKey() {
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
{:else if needsCredentials}
	<form
		class="mx-auto mt-16 w-full max-w-sm rounded-card border border-line bg-surface p-7"
		onsubmit={signIn}
		autocomplete="off"
	>
		<h2 class="text-2xl font-semibold">{m.terminal_credentials_title({ name: device.name })}</h2>
		<p class="mt-1 text-sm text-ink-2">{m.terminal_credentials_hint()}</p>
		<!-- VNC servers mostly know only a password. -->
		<label class="mt-4 block text-sm font-medium" for="target-username-{device.id}">
			{device.protocol === 'vnc' ? m.credentials_username_optional() : m.field_username()}
		</label>
		<input
			id="target-username-{device.id}"
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
			required={device.protocol !== 'vnc'}
			spellcheck="false"
			bind:value={username}
		/>
		<label class="mt-3 block text-sm font-medium" for="target-password-{device.id}">
			{m.field_password()}
		</label>
		<input
			id="target-password-{device.id}"
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
{:else}
	<div class="relative h-full min-h-80">
		{#key attempt}
			{#if graphical}
				<DisplayView
					deviceId={device.id}
					name={device.name}
					{credentials}
					{visible}
					{onevent}
					{onfailure}
					{onend}
				/>
			{:else}
				<TerminalView deviceId={device.id} {credentials} {visible} {onevent} {onend} />
			{/if}
		{/key}

		{#if phase === 'closed' || phase === 'failed'}
			<div class="absolute inset-0 flex items-center justify-center bg-page/75 p-6">
				<div
					class="w-full max-w-lg rounded-card border bg-surface p-5 text-sm {changed
						? 'border-critical/50'
						: 'border-line'}"
					role={phase === 'failed' ? 'alert' : 'status'}
				>
					<p class="flex items-start gap-2">
						{#if changed}
							<ShieldAlert size={18} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
						{:else if phase === 'failed'}
							<CircleAlert size={18} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
						{:else}
							<CircleMinus size={18} class="mt-0.5 shrink-0 text-ink-3" aria-hidden="true" />
						{/if}
						<span>
							{#if changed}
								{(status.kind === 'error' && status.code === 'certificate_changed'
									? m.display_certificate_changed
									: m.terminal_host_key_changed)({
									name: device.name,
									expected: param('expected'),
									presented: param('presented')
								})}
							{:else}
								{text}
							{/if}
						</span>
					</p>
					{#if status.kind === 'failed' && status.detail}
						<!-- guacd's own words, for the administrator. -->
						<p class="mt-2 pl-6.5 text-xs text-ink-3">
							{m.display_detail({ detail: status.detail })}
						</p>
					{/if}
					<div class="mt-4 flex flex-wrap gap-2 pl-6.5">
						{#if changed && allows(device.role, 'edit')}
							<button type="button" class={button} onclick={trustNewKey}>
								{status.kind === 'error' && status.code === 'certificate_changed'
									? m.display_trust_new_certificate()
									: m.terminal_trust_new_key()}
							</button>
						{/if}
						<button type="button" class={button} onclick={reconnect}>
							<RotateCw size={15} aria-hidden="true" />
							{m.terminal_reconnect()}
						</button>
					</div>
				</div>
			</div>
		{/if}
	</div>
{/if}
