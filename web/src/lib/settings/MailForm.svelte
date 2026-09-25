<script lang="ts">
	/**
	 * The mail server (#145), on the settings page and in the setup wizard.
	 * "Send test mail" sends with the form as it is, stored or not, and says
	 * where it failed. A password never goes over an unencrypted connection,
	 * and it is never shown again.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import {
		DEFAULT_PORTS,
		loadMailServer,
		removeMailServer,
		saveMailServer,
		sendTestMail,
		type MailInput,
		type Security,
		type SendFailure,
		type SendStep
	} from '$lib/api/mail';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let {
		removable = false,
		recipient = '',
		onsaved
	}: {
		/** Offers to remove a stored server. */
		removable?: boolean;
		/** Where the test mail goes unless changed: the administrator. */
		recipient?: string;
		onsaved?: () => void;
	} = $props();

	let host = $state('');
	let security = $state<Security>('starttls');
	let port = $state(DEFAULT_PORTS.starttls);
	let username = $state('');
	let password = $state('');
	let fromAddress = $state('');
	let fromName = $state('remotehub');
	let caPem = $state('');
	/** A server is stored; with a user name, a password is too. */
	let stored = $state(false);
	let storedUsername = $state<string | null>(null);
	let testTo = $state('');

	let busy = $state(false);
	let error = $state<string | null>(null);
	let saved = $state(false);
	let sent = $state(false);
	let failure = $state<SendFailure | null>(null);
	let removing = $state(false);

	$effect(() => {
		testTo = recipient;
		loadMailServer().then((result) => {
			if (!result.ok) {
				error = errorMessage(result.code);
				return;
			}
			const server = result.data.server;
			stored = server !== null;
			if (!server) return;
			host = server.host;
			security = server.security;
			port = server.port;
			username = server.username ?? '';
			storedUsername = server.username;
			fromAddress = server.from_address;
			fromName = server.from_name;
			caPem = server.ca_pem ?? '';
		});
	});

	/** A new kind of security brings its usual port, unless one was typed. */
	function secure(next: Security) {
		if (port === DEFAULT_PORTS[security]) port = DEFAULT_PORTS[next];
		security = next;
		if (next === 'none') username = password = '';
	}

	const input = (): MailInput => ({
		host: host.trim(),
		port,
		security,
		username: username.trim() || null,
		password: password || null,
		from_address: fromAddress.trim(),
		from_name: fromName.trim(),
		ca_pem: caPem.trim() || null
	});

	function clear() {
		error = failure = null;
		saved = sent = false;
	}

	async function test() {
		clear();
		busy = true;
		const result = await sendTestMail(input(), testTo.trim(), getLocale());
		busy = false;
		if (!result.ok) error = problemMessage(result);
		else if (result.data.failure) failure = result.data.failure;
		else sent = true;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		clear();
		busy = true;
		const result = await saveMailServer(input());
		busy = false;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		saved = stored = true;
		storedUsername = username.trim() || null;
		password = '';
		onsaved?.();
	}

	async function remove() {
		clear();
		const result = await removeMailServer();
		removing = false;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		stored = false;
	}

	const STEPS: Record<SendStep, () => string> = {
		not_configured: m.mail_step_not_configured,
		address: m.mail_step_address,
		connect: m.mail_step_connect,
		tls: m.mail_step_tls,
		sign_in: m.mail_step_sign_in,
		rejected: m.mail_step_rejected,
		other: m.mail_step_other
	};
	const SECURITY: Record<Security, () => string> = {
		tls: m.mail_security_tls,
		starttls: m.mail_security_starttls,
		none: m.mail_security_none
	};
	const passwordKept = $derived(stored && !!storedUsername && username.trim() === storedUsername);

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-4 block text-sm font-medium';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
</script>

<form onsubmit={save}>
	<div class="grid gap-x-4 sm:grid-cols-[1fr_8rem]">
		<div>
			<label class={label} for="mail-host">{m.mail_host()}</label>
			<input id="mail-host" class="{field} font-mono" required bind:value={host} />
		</div>
		<div>
			<label class={label} for="mail-port">{m.field_port()}</label>
			<input
				id="mail-port"
				class={field}
				type="number"
				min="1"
				max="65535"
				required
				bind:value={port}
			/>
		</div>
	</div>

	<fieldset class="mt-4">
		<legend class="text-sm font-medium">{m.mail_security()}</legend>
		<div class="mt-1 flex flex-wrap gap-4 text-sm">
			{#each ['tls', 'starttls', 'none'] as const as kind (kind)}
				<label class="flex items-center gap-2">
					<input
						type="radio"
						name="mail-security"
						checked={security === kind}
						onchange={() => secure(kind)}
					/>
					{SECURITY[kind]()}
				</label>
			{/each}
		</div>
		{#if security === 'none'}
			<p class="mt-1 text-xs text-ink-3">{m.mail_security_none_hint()}</p>
		{/if}
	</fieldset>

	{#if security !== 'none'}
		<label class={label} for="mail-username">{m.mail_username()}</label>
		<input id="mail-username" class={field} autocomplete="off" bind:value={username} />
		<label class={label} for="mail-password">{m.mail_password()}</label>
		<input
			id="mail-password"
			class={field}
			type="password"
			autocomplete="new-password"
			bind:value={password}
		/>
		{#if passwordKept}
			<p class="mt-1 text-xs text-ink-3">{m.directory_password_stored()}</p>
		{/if}
	{/if}

	<label class={label} for="mail-from">{m.mail_from_address()}</label>
	<input id="mail-from" class={field} type="email" required bind:value={fromAddress} />
	<label class={label} for="mail-from-name">{m.mail_from_name()}</label>
	<input id="mail-from-name" class={field} bind:value={fromName} />

	<details class="mt-4">
		<summary class="cursor-pointer text-sm text-ink-2">{m.directory_more()}</summary>
		<label class={label} for="mail-ca">{m.mail_ca()}</label>
		<textarea id="mail-ca" class="{field} h-28 font-mono text-xs" bind:value={caPem}></textarea>
		<p class="mt-1 text-xs text-ink-3">{m.mail_ca_hint()}</p>
	</details>

	<div class="mt-5 flex flex-wrap items-end gap-2">
		<div class="min-w-56 flex-1">
			<label class="block text-sm font-medium" for="mail-test-to">{m.mail_test_to()}</label>
			<input id="mail-test-to" class={field} type="email" bind:value={testTo} />
		</div>
		<button type="button" class={button} disabled={busy || !testTo.trim()} onclick={test}>
			{m.mail_test()}
		</button>
	</div>

	<div class="mt-5 flex flex-wrap gap-2">
		<button type="submit" class={primary} disabled={busy}>{m.action_save()}</button>
		{#if removable && stored}
			<button
				type="button"
				class="{button} ml-auto text-critical"
				onclick={() => (removing = true)}
			>
				{m.mail_remove()}
			</button>
		{/if}
	</div>
</form>

{#if removing}
	<div class="mt-4 rounded-lg border border-line p-3 text-sm" role="alert">
		<p>{m.mail_remove_confirm()}</p>
		<div class="mt-3 flex gap-2">
			<button type="button" class="{button} text-critical" onclick={remove}>
				{m.mail_remove()}
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

{#if sent || saved}
	<p class="mt-4 flex items-center gap-2 text-sm" role="status" data-testid="mail-done">
		<CircleCheck size={16} class="text-ok" aria-hidden="true" />
		{sent ? m.mail_test_sent({ to: testTo.trim() }) : m.mail_saved()}
	</p>
{/if}

{#if failure}
	<div class="mt-4 rounded-lg border border-line p-3 text-sm" role="alert">
		<p class="flex items-start gap-2" data-testid="mail-failure">
			<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
			{STEPS[failure.step]()}
		</p>
		{#if failure.detail}
			<p class="mt-1 font-mono text-xs break-all text-ink-3">{failure.detail}</p>
		{/if}
	</div>
{/if}
