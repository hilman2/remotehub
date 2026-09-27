<script lang="ts">
	/**
	 * The popup of the extension (#201, ADR 0017): the logins of the open page,
	 * to fill with a click, and a search across all logins, to copy from. It
	 * lives in the browser's own frame, where the page can neither cover nor
	 * restyle it; nothing is ever drawn into the page.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Copy from '@lucide/svelte/icons/copy';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Lock from '@lucide/svelte/icons/lock';
	import LogIn from '@lucide/svelte/icons/log-in';
	import Search from '@lucide/svelte/icons/search';
	import Settings from '@lucide/svelte/icons/settings';
	import Timer from '@lucide/svelte/icons/timer';
	import User from '@lucide/svelte/icons/user';
	import Logo from '$lib/components/Logo.svelte';
	import { m } from '$lib/paraglide/messages';
	import { CLIPBOARD_SECONDS, clearLater } from './clipboard';
	import {
		allLogins,
		currentCode,
		fillCode,
		fillLogin,
		forPage,
		search,
		tabOrigin,
		type Login,
		type Logins,
		type Outcome
	} from './logins';
	import { message } from './messages';
	import { copyPassword } from './server';
	import { pageHost } from './site';
	import { connection, lockVault, type Connection } from './store';

	let tab = $state<chrome.tabs.Tab | undefined>();
	let connected = $state<Connection | null>(null);
	let data = $state<Logins | null>(null);
	let loaded = $state(false);
	let query = $state('');
	let busy = $state(false);
	let status = $state<{ ok: boolean; text: string } | null>(null);
	let shortcut = $state('');

	const origin = $derived(tabOrigin(tab));
	const host = $derived(pageHost(tab?.url));
	const matching = $derived(data ? forPage(data.logins, origin) : []);
	const found = $derived(data ? search(data.logins, query) : []);

	$effect(() => {
		(async () => {
			// Opened from the toolbar, the popup acts on the active tab. Opened
			// as a page of its own, as the end-to-end tests do, its query names
			// the tab; only the extension itself can open its pages.
			const named = Number(new URLSearchParams(location.search).get('tab'));
			[tab] =
				Number.isInteger(named) && named > 0
					? [await chrome.tabs.get(named)]
					: await chrome.tabs.query({ active: true, currentWindow: true });
			connected = await connection();
			if (connected) {
				data = await allLogins();
				// The server ended the session: the extension forgot it meanwhile.
				connected = await connection();
			}
			const [command] = (await chrome.commands.getAll()).filter((c) => c.name === 'fill');
			shortcut = command?.shortcut ?? '';
			loaded = true;
		})();
	});

	function show(outcome: Outcome | { ok: true; text: string }) {
		if ('text' in outcome) status = { ok: true, text: outcome.text };
		else if (outcome.ok)
			status = {
				ok: true,
				text:
					outcome.filled.password || outcome.filled.code
						? m.extension_filled()
						: m.extension_filled_user()
			};
		else status = { ok: false, text: message(outcome.code) };
	}

	async function fill(login: Login) {
		if (!tab) return;
		busy = true;
		const outcome = await fillLogin(tab, login);
		busy = false;
		show(outcome);
		// Done unless a one-time code follows on the next page.
		if (outcome.ok && outcome.filled.password && !login.hasTotp) window.close();
	}

	async function fillTheCode(login: Login) {
		if (!tab) return;
		busy = true;
		const outcome = await fillCode(tab, login);
		busy = false;
		show(outcome);
		if (outcome.ok) window.close();
	}

	/** Copies `text`; a secret leaves the clipboard again after a while. */
	async function copy(text: string, secret = false) {
		try {
			await navigator.clipboard.writeText(text);
		} catch {
			status = { ok: false, text: m.extension_copy_failed() };
			return;
		}
		if (secret) await clearLater();
		show({
			ok: true,
			text: secret
				? m.extension_copied_secret({ seconds: CLIPBOARD_SECONDS })
				: m.extension_copied()
		});
	}

	async function copySecret(login: Login, what: 'password' | 'code') {
		busy = true;
		try {
			if (what === 'code') {
				const code = await currentCode(login, 'copy');
				if (code.ok) await copy(code.code, true);
				else status = { ok: false, text: message(code.code) };
			} else if (login.kind === 'personal') {
				await copy(login.password ?? '', true);
			} else {
				const answer = await copyPassword(login.id);
				if (answer.ok) await copy(answer.data.password, true);
				else status = { ok: false, text: message(answer.code) };
			}
		} finally {
			busy = false;
		}
	}

	async function lock() {
		await lockVault();
		data = await allLogins();
		show({ ok: true, text: m.extension_locked() });
	}

	const openOptions = (hash = '') =>
		chrome.tabs.create({ url: chrome.runtime.getURL(`options.html${hash}`) });

	const icon =
		'inline-flex size-8 items-center justify-center rounded-lg border border-line bg-surface text-ink-2 hover:bg-surface-2 hover:text-ink disabled:opacity-50';
</script>

{#snippet row(login: Login, fillable: boolean)}
	<li class="flex flex-col gap-1.5 rounded-xl border border-line bg-surface p-2.5">
		<div class="flex items-center gap-2">
			<div class="min-w-0 flex-1">
				<p class="truncate font-medium">{login.name}</p>
				<p class="truncate text-xs text-ink-2">
					{login.username}
					<span class="text-ink-3">
						· {login.kind === 'personal' ? m.extension_personal() : m.extension_shared()}
					</span>
				</p>
			</div>
			{#if fillable}
				<button
					type="button"
					class="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:opacity-90 disabled:opacity-50"
					disabled={busy}
					aria-label={m.extension_fill_login({ name: login.name })}
					onclick={() => fill(login)}
				>
					<LogIn size={14} aria-hidden="true" />
					{m.extension_fill()}
				</button>
			{/if}
		</div>
		<div class="flex gap-1.5">
			<button
				type="button"
				class={icon}
				disabled={busy}
				title={m.extension_copy_username({ name: login.name })}
				aria-label={m.extension_copy_username({ name: login.name })}
				onclick={() => copy(login.username)}
			>
				<User size={14} aria-hidden="true" />
			</button>
			<button
				type="button"
				class={icon}
				disabled={busy}
				title={m.extension_copy_password({ name: login.name })}
				aria-label={m.extension_copy_password({ name: login.name })}
				onclick={() => copySecret(login, 'password')}
			>
				<KeyRound size={14} aria-hidden="true" />
			</button>
			{#if login.hasTotp}
				{#if fillable}
					<button
						type="button"
						class={icon}
						disabled={busy}
						title={m.extension_fill_code({ name: login.name })}
						aria-label={m.extension_fill_code({ name: login.name })}
						onclick={() => fillTheCode(login)}
					>
						<Timer size={14} aria-hidden="true" />
					</button>
				{/if}
				<button
					type="button"
					class={icon}
					disabled={busy}
					title={m.extension_copy_code({ name: login.name })}
					aria-label={m.extension_copy_code({ name: login.name })}
					onclick={() => copySecret(login, 'code')}
				>
					<Copy size={14} aria-hidden="true" />
				</button>
			{/if}
		</div>
	</li>
{/snippet}

<main class="flex w-[360px] flex-col gap-3 bg-page p-3 text-sm text-ink">
	<header class="flex items-center gap-2">
		<Logo />
		<div class="min-w-0 flex-1">
			<p class="font-display font-bold tracking-tight">{m.extension_name()}</p>
			{#if connected}
				<p class="truncate text-xs text-ink-2">{connected.displayName}</p>
			{/if}
		</div>
		<button
			type="button"
			class={icon}
			title={m.extension_settings()}
			aria-label={m.extension_settings()}
			onclick={() => openOptions()}
		>
			<Settings size={14} aria-hidden="true" />
		</button>
	</header>

	{#if !loaded}
		<p class="text-ink-2" role="status">{m.extension_loading()}</p>
	{:else if !connected}
		<p class="text-ink-2">{m.extension_not_connected()}</p>
		<button
			type="button"
			class="self-start rounded-lg bg-accent px-3 py-1.5 font-medium text-accent-ink"
			onclick={() => openOptions()}
		>
			{m.extension_set_up()}
		</button>
	{:else}
		<section class="flex flex-col gap-2" aria-labelledby="popup-page">
			<h1 id="popup-page" class="text-xs text-ink-2">
				{m.extension_page()}
				<span class="font-mono text-ink">{host ?? '–'}</span>
			</h1>
			{#if !origin}
				<p class="text-ink-2">{m.extension_no_page()}</p>
			{:else if matching.length === 0}
				<p class="text-ink-2">{m.extension_matches_none()}</p>
			{:else}
				<ul class="flex flex-col gap-2">
					{#each matching as login (login.key)}
						{@render row(login, true)}
					{/each}
				</ul>
			{/if}
		</section>

		{#if status}
			<p class="flex items-start gap-1.5" role={status.ok ? 'status' : 'alert'}>
				{#if status.ok}
					<CircleCheck size={16} class="mt-0.5 shrink-0 text-ok" aria-hidden="true" />
				{:else}
					<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{/if}
				{status.text}
			</p>
		{/if}
		{#if data?.error}
			<p class="flex items-start gap-1.5" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{message(data.error)}
			</p>
		{/if}

		{#if data?.personal === 'locked'}
			<div class="flex items-center gap-2 rounded-xl border border-line bg-surface-2 p-2.5">
				<Lock size={16} class="shrink-0 text-ink-2" aria-hidden="true" />
				<p class="flex-1 text-xs">{m.extension_personal_locked()}</p>
				<button
					type="button"
					class="rounded-lg border border-line-strong bg-surface px-2.5 py-1 text-xs hover:bg-surface-2"
					onclick={() => openOptions('#unlock')}
				>
					{m.extension_unlock()}
				</button>
			</div>
		{/if}

		<section class="flex flex-col gap-2" aria-labelledby="popup-search">
			<h2 id="popup-search" class="sr-only">{m.extension_search()}</h2>
			<div class="relative">
				<Search
					size={14}
					class="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3"
					aria-hidden="true"
				/>
				<input
					type="search"
					class="h-9 w-full rounded-lg border border-line-strong bg-surface pr-2 pl-8"
					placeholder={m.extension_search()}
					aria-label={m.extension_search()}
					bind:value={query}
				/>
			</div>
			{#if query.trim()}
				{#if found.length === 0}
					<p class="text-ink-2">{m.extension_search_none()}</p>
				{:else}
					<p class="text-xs text-ink-3">{m.extension_search_hint()}</p>
					<ul class="flex max-h-64 flex-col gap-2 overflow-y-auto">
						{#each found as login (login.key)}
							{@render row(
								login,
								matching.some((match) => match.key === login.key)
							)}
						{/each}
					</ul>
				{/if}
			{/if}
		</section>

		<footer class="flex items-center gap-2 border-t border-line pt-2 text-xs text-ink-3">
			<p class="flex-1">
				{#if shortcut}{m.extension_shortcut({ keys: shortcut })}{/if}
			</p>
			{#if data?.personal === 'open'}
				<button
					type="button"
					class="inline-flex items-center gap-1 rounded-lg px-2 py-1 hover:bg-surface-2 hover:text-ink"
					onclick={lock}
				>
					<Lock size={12} aria-hidden="true" />
					{m.extension_lock()}
				</button>
			{/if}
		</footer>
	{/if}
</main>
