<script lang="ts">
	/**
	 * The extension's settings page (#201, ADR 0017), in a tab of its own:
	 * connecting to remotehub and unlocking the personal logins. Both open
	 * windows of the browser (the sign-in window, the passkey prompt), which
	 * would close the popup and end what it had started.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import Lock from '@lucide/svelte/icons/lock';
	import Logo from '$lib/components/Logo.svelte';
	import { m } from '$lib/paraglide/messages';
	import {
		unlockWithPassphrase,
		unlockWithPasskey,
		unlockWithRecovery,
		type StoredVault
	} from '$lib/vault/vault';
	import { connectTo } from './connecting';
	import { message } from './messages';
	import { loadPersonal, signOut } from './server';
	import {
		connection,
		lockVault,
		normaliseServer,
		saveServerAddress,
		saveVaultKey,
		serverAddress,
		serverPattern,
		vaultKey,
		type Connection
	} from './store';

	let address = $state('');
	let managed = $state(false);
	let connected = $state<Connection | null>(null);
	let vault = $state<StoredVault | null>(null);
	let open = $state(false);
	let passphrase = $state('');
	let recovery = $state('');
	let busy = $state(false);
	let status = $state<{ ok: boolean; text: string } | null>(null);

	const hasUnlock = (kind: string) => !!vault?.unlocks.some((unlock) => unlock.kind === kind);

	async function load() {
		const saved = await serverAddress();
		address = saved.url ?? '';
		managed = saved.managed;
		connected = await connection();
		vault = null;
		if (connected) {
			const personal = await loadPersonal();
			if (personal.ok) vault = personal.data;
			connected = await connection();
			open = (await vaultKey()) !== null;
		}
	}

	$effect(() => {
		load();
	});

	async function connect(event: SubmitEvent) {
		event.preventDefault();
		status = null;
		const server = normaliseServer(address);
		if (!server) {
			status = { ok: false, text: m.extension_server_invalid() };
			return;
		}
		// Asked for here, from the click: Chromium grants host access only
		// on a user's gesture, and the extension needs it for remotehub alone.
		const granted = await chrome.permissions.request({ origins: [serverPattern(server)] });
		if (!granted) {
			status = { ok: false, text: m.extension_permission_denied() };
			return;
		}
		if (!managed) await saveServerAddress(server);
		busy = true;
		const outcome = await connectTo(server);
		busy = false;
		if (!outcome.ok) status = { ok: false, text: message(outcome.code) };
		await load();
	}

	async function disconnect() {
		busy = true;
		await signOut();
		busy = false;
		await load();
	}

	async function unlocked(key: CryptoKey | null) {
		if (!key) {
			status = { ok: false, text: m.extension_unlock_wrong() };
			return;
		}
		await saveVaultKey(key);
		status = { ok: true, text: m.extension_unlocked() };
		open = true;
	}

	async function withPasskey() {
		if (!vault || !connected) return;
		busy = true;
		status = null;
		try {
			// The passkeys belong to remotehub's host; the extension may use
			// them there because it has access to that host.
			await unlocked(await unlockWithPasskey(vault, new URL(connected.server).hostname));
		} catch {
			status = { ok: false, text: m.extension_unlock_wrong() };
		} finally {
			busy = false;
		}
	}

	async function withPassphrase(event: SubmitEvent) {
		event.preventDefault();
		if (!vault) return;
		busy = true;
		status = null;
		const key = await unlockWithPassphrase(vault, passphrase);
		passphrase = '';
		busy = false;
		await unlocked(key);
	}

	async function withRecovery(event: SubmitEvent) {
		event.preventDefault();
		if (!vault) return;
		busy = true;
		status = null;
		const key = await unlockWithRecovery(vault, recovery);
		recovery = '';
		busy = false;
		await unlocked(key);
	}

	async function lock() {
		await lockVault();
		open = false;
		status = { ok: true, text: m.extension_locked() };
	}

	const card = 'flex flex-col gap-3 rounded-card border border-line bg-surface p-6';
	const button =
		'inline-flex items-center gap-2 self-start rounded-xl border border-line-strong bg-surface px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-60';
	const primary =
		'inline-flex items-center gap-2 self-start rounded-xl bg-accent px-4 py-2 text-sm font-medium text-accent-ink hover:opacity-90 disabled:opacity-60';
	const field = 'h-11 w-full max-w-md rounded-xl border border-line-strong bg-page px-3';
</script>

<main class="mx-auto flex min-h-dvh max-w-2xl flex-col gap-5 bg-page p-8 text-ink">
	<header class="flex items-center gap-2.5">
		<Logo />
		<h1 class="font-display text-2xl font-bold tracking-tight">{m.extension_options_title()}</h1>
	</header>
	<p class="text-sm text-ink-2">{m.extension_description()}</p>

	{#if status}
		<p class="flex items-start gap-2 text-sm" role={status.ok ? 'status' : 'alert'}>
			{#if status.ok}
				<CircleCheck size={18} class="mt-0.5 shrink-0 text-ok" aria-hidden="true" />
			{:else}
				<CircleAlert size={18} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
			{/if}
			{status.text}
		</p>
	{/if}

	<section class={card} aria-labelledby="options-server">
		<h2 id="options-server" class="text-lg font-semibold">{m.extension_connection()}</h2>
		{#if connected}
			<p class="flex items-center gap-2 text-sm">
				<CircleCheck size={16} class="shrink-0 text-ok" aria-hidden="true" />
				{m.extension_connected_as({ name: connected.displayName, server: connected.server })}
			</p>
			<button type="button" class={button} disabled={busy} onclick={disconnect}>
				{m.extension_disconnect()}
			</button>
		{:else}
			<form class="flex flex-col gap-3" onsubmit={connect}>
				<label class="text-sm font-medium" for="options-address">{m.extension_server()}</label>
				<input
					id="options-address"
					class={field}
					type="url"
					required
					readonly={managed}
					autocomplete="url"
					bind:value={address}
				/>
				{#if managed}
					<p class="text-xs text-ink-2">{m.extension_server_managed()}</p>
				{/if}
				<button type="submit" class={primary} disabled={busy}>
					{busy ? m.extension_connecting() : m.extension_connect()}
				</button>
			</form>
		{/if}
	</section>

	{#if connected}
		<section id="unlock" class={card} aria-labelledby="options-personal">
			<h2 id="options-personal" class="text-lg font-semibold">{m.extension_unlock_title()}</h2>
			{#if !vault || vault.unlocks.length === 0}
				<p class="text-sm text-ink-2">{m.extension_no_personal()}</p>
			{:else if open}
				<p class="flex items-center gap-2 text-sm">
					<CircleCheck size={16} class="shrink-0 text-ok" aria-hidden="true" />
					{m.extension_unlocked()}
				</p>
				<button type="button" class={button} onclick={lock}>
					<Lock size={16} aria-hidden="true" />
					{m.extension_lock()}
				</button>
			{:else}
				<p class="text-sm text-ink-2">{m.extension_unlock_hint()}</p>
				{#if hasUnlock('passkey')}
					<button type="button" class={primary} disabled={busy} onclick={withPasskey}>
						<Fingerprint size={16} aria-hidden="true" />
						{m.extension_unlock_passkey()}
					</button>
				{/if}
				{#if hasUnlock('passphrase')}
					<form class="flex flex-wrap items-end gap-3" onsubmit={withPassphrase}>
						<div class="flex min-w-0 flex-1 flex-col gap-1">
							<label class="text-sm font-medium" for="options-passphrase">
								{m.extension_unlock_passphrase()}
							</label>
							<input
								id="options-passphrase"
								class={field}
								type="password"
								autocomplete="off"
								required
								bind:value={passphrase}
							/>
						</div>
						<button type="submit" class={button} disabled={busy}>
							{m.extension_unlock_submit()}
						</button>
					</form>
				{/if}
				{#if hasUnlock('recovery')}
					<form class="flex flex-wrap items-end gap-3" onsubmit={withRecovery}>
						<div class="flex min-w-0 flex-1 flex-col gap-1">
							<label class="text-sm font-medium" for="options-recovery">
								{m.extension_unlock_recovery()}
							</label>
							<input
								id="options-recovery"
								class="{field} font-mono"
								autocomplete="off"
								spellcheck="false"
								required
								bind:value={recovery}
							/>
						</div>
						<button type="submit" class={button} disabled={busy}>
							{m.extension_unlock_submit()}
						</button>
					</form>
				{/if}
			{/if}
		</section>
	{/if}
</main>
