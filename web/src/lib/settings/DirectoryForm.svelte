<script lang="ts">
	/**
	 * The directory connection (#144), on the settings page and in the setup
	 * wizard. "Test connection" names the step that fails; an unknown CA
	 * comes with the certificate the server presented, to trust after
	 * comparing its fingerprint. Saving checks again and stores only what
	 * passes; the password is never shown again.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import ShieldQuestion from '@lucide/svelte/icons/shield-question';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import {
		checkConnection,
		loadConnection,
		removeConnection,
		saveConnection,
		type CheckFailure,
		type CheckReason,
		type CheckStep,
		type ConnectionInput,
		type Found
	} from '$lib/api/directory';
	import { m } from '$lib/paraglide/messages';

	let {
		removable = false,
		onsaved
	}: {
		/** Offers to remove a stored connection. */
		removable?: boolean;
		onsaved?: (found: Found) => void;
	} = $props();

	let url = $state('ldaps://');
	let starttls = $state(false);
	let caPem = $state('');
	let bindDn = $state('');
	let password = $state('');
	let baseDn = $state('');
	let userFilter = $state('');
	let timeout = $state(10);
	/** A connection is stored, and with it a password. */
	let stored = $state(false);
	let more = $state(false);

	let busy = $state(false);
	let found = $state<Found | null>(null);
	let failure = $state<CheckFailure | null>(null);
	let saved = $state(false);
	let error = $state<string | null>(null);
	let removing = $state(false);

	$effect(() => {
		loadConnection().then((result) => {
			if (!result.ok) {
				error = errorMessage(result.code);
				return;
			}
			const connection = result.data.connection;
			stored = connection !== null;
			if (!connection) return;
			url = connection.url;
			starttls = connection.starttls;
			caPem = connection.ca_pem ?? '';
			bindDn = connection.bind_dn;
			baseDn = connection.base_dn;
			userFilter = connection.user_filter ?? '';
			timeout = connection.timeout_seconds;
			more = !!connection.ca_pem || !!connection.user_filter || connection.starttls;
		});
	});

	const input = (): ConnectionInput => ({
		url: url.trim(),
		starttls,
		ca_pem: caPem.trim() || null,
		bind_dn: bindDn.trim(),
		password: password || null,
		base_dn: baseDn.trim(),
		user_filter: userFilter.trim() || null,
		timeout_seconds: timeout
	});

	function clear() {
		found = failure = error = null;
		saved = false;
	}

	async function check() {
		clear();
		busy = true;
		const result = await checkConnection(input());
		busy = false;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		found = result.data.found ?? null;
		failure = result.data.failure ?? null;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		clear();
		busy = true;
		const result = await saveConnection(input());
		busy = false;
		if (result.ok) {
			found = result.data;
			saved = stored = true;
			password = '';
			onsaved?.(result.data);
		} else if (result.code === 'directory_check_failed') {
			failure = {
				step: result.params.step as CheckStep,
				reason: result.params.reason as CheckReason,
				detail: String(result.params.detail ?? '')
			};
		} else {
			error = problemMessage(result);
		}
	}

	/** Adds the presented certificate to the trusted ones and checks again. */
	async function trust(pem: string) {
		caPem = caPem.trim() ? `${caPem.trim()}\n${pem}` : pem;
		more = true;
		await check();
	}

	async function remove() {
		clear();
		const result = await removeConnection();
		removing = false;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		stored = false;
		password = '';
	}

	const STEPS: Record<CheckStep, () => string> = {
		resolve: m.directory_step_resolve,
		connect: m.directory_step_connect,
		tls: m.directory_step_tls,
		bind: m.directory_step_bind,
		search: m.directory_step_search
	};
	const REASONS: Record<CheckReason, () => string> = {
		not_found: m.directory_reason_not_found,
		refused: m.directory_reason_refused,
		timeout: m.directory_reason_timeout,
		start_tls_refused: m.directory_reason_start_tls_refused,
		unknown_ca: m.directory_reason_unknown_ca,
		name_mismatch: m.directory_reason_name_mismatch,
		expired: m.directory_reason_expired,
		invalid_credentials: m.directory_reason_invalid_credentials,
		no_such_base: m.directory_reason_no_such_base,
		other: m.directory_reason_other
	};

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-4 block text-sm font-medium';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
</script>

<form onsubmit={save}>
	<label class={label} for="directory-url">{m.directory_url()}</label>
	<input id="directory-url" class="{field} font-mono" required bind:value={url} />
	<p class="mt-1 text-xs text-ink-3">{m.directory_url_hint()}</p>

	<label class={label} for="directory-bind-dn">{m.directory_bind_dn()}</label>
	<input id="directory-bind-dn" class={field} required autocomplete="off" bind:value={bindDn} />
	<p class="mt-1 text-xs text-ink-3">{m.directory_bind_dn_hint()}</p>

	<label class={label} for="directory-password">{m.directory_password()}</label>
	<input
		id="directory-password"
		class={field}
		type="password"
		autocomplete="new-password"
		required={!stored}
		bind:value={password}
	/>
	{#if stored}
		<p class="mt-1 text-xs text-ink-3">{m.directory_password_stored()}</p>
	{/if}

	<label class={label} for="directory-base-dn">{m.directory_base_dn()}</label>
	<input id="directory-base-dn" class="{field} font-mono" required bind:value={baseDn} />

	<details class="mt-4" bind:open={more}>
		<summary class="cursor-pointer text-sm text-ink-2">{m.directory_more()}</summary>
		<label class="mt-3 flex items-center gap-2 text-sm">
			<input type="checkbox" bind:checked={starttls} />
			{m.directory_starttls()}
		</label>
		<label class={label} for="directory-ca">{m.directory_ca()}</label>
		<textarea id="directory-ca" class="{field} h-28 font-mono text-xs" bind:value={caPem}
		></textarea>
		<p class="mt-1 text-xs text-ink-3">{m.directory_ca_hint()}</p>
		<label class={label} for="directory-filter">{m.directory_user_filter()}</label>
		<input id="directory-filter" class="{field} font-mono" bind:value={userFilter} />
		<p class="mt-1 text-xs text-ink-3">{m.directory_user_filter_hint()}</p>
		<label class={label} for="directory-timeout">{m.directory_timeout()}</label>
		<input
			id="directory-timeout"
			class="{field} w-24"
			type="number"
			min="1"
			max="60"
			bind:value={timeout}
		/>
	</details>

	<div class="mt-5 flex flex-wrap gap-2">
		<button type="button" class={button} disabled={busy} onclick={check}>
			{m.directory_check()}
		</button>
		<button type="submit" class={primary} disabled={busy}>{m.action_save()}</button>
		{#if removable && stored}
			<button
				type="button"
				class="{button} ml-auto text-critical"
				onclick={() => (removing = true)}
			>
				{m.directory_remove()}
			</button>
		{/if}
	</div>
</form>

{#if removing}
	<div class="mt-4 rounded-lg border border-line p-3 text-sm" role="alert">
		<p>{m.directory_remove_confirm()}</p>
		<div class="mt-3 flex gap-2">
			<button type="button" class="{button} text-critical" onclick={remove}>
				{m.directory_remove()}
			</button>
			<button type="button" class={button} onclick={() => (removing = false)}>
				{m.action_cancel()}
			</button>
		</div>
	</div>
{/if}

{#if error}
	<p class="mt-4 flex items-center gap-2 text-sm text-critical" role="alert">
		<CircleAlert size={16} aria-hidden="true" />
		{error}
	</p>
{/if}

{#if found}
	<p class="mt-4 flex items-start gap-2 text-sm" role="status" data-testid="directory-found">
		<CircleCheck size={16} class="mt-0.5 shrink-0 text-ok" aria-hidden="true" />
		<span>
			{saved ? m.directory_saved() : ''}
			{(found.more ? m.directory_found_more : m.directory_found)({
				count: found.users,
				names: found.sample.join(', ')
			})}
		</span>
	</p>
{/if}

{#if failure}
	<div class="mt-4 rounded-lg border border-line p-3 text-sm" role="alert">
		<p class="flex items-start gap-2" data-testid="directory-failure">
			<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
			<span>
				{m.directory_failed_at({ step: STEPS[failure.step]() })}
				{REASONS[failure.reason]()}
			</span>
		</p>
		{#if failure.detail}
			<p class="mt-1 font-mono text-xs break-all text-ink-3">{failure.detail}</p>
		{/if}
		{#if failure.ca}
			{@const ca = failure.ca}
			<div class="mt-3 border-t border-line pt-3">
				<p class="flex items-center gap-2 font-medium">
					<ShieldQuestion size={16} aria-hidden="true" />
					{m.directory_presented()}
				</p>
				<dl class="mt-2 grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 text-xs">
					<dt class="text-ink-2">{m.directory_subject()}</dt>
					<dd class="font-mono break-all">{ca.subject}</dd>
					<dt class="text-ink-2">{m.directory_fingerprint()}</dt>
					<dd class="font-mono break-all" data-testid="directory-fingerprint">{ca.fingerprint}</dd>
				</dl>
				<p class="mt-2 text-xs text-ink-2">
					{ca.authority ? m.directory_trust_ca_hint() : m.directory_trust_server_hint()}
				</p>
				<button type="button" class="{button} mt-3" disabled={busy} onclick={() => trust(ca.pem)}>
					{ca.authority ? m.directory_trust_ca() : m.directory_trust_server()}
				</button>
			</div>
		{/if}
	</div>
{/if}
