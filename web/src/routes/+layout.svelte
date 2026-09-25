<script lang="ts">
	import './layout.css';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Siren from '@lucide/svelte/icons/siren';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import favicon from '$lib/assets/favicon.svg';
	import LocaleSwitch from '$lib/components/LocaleSwitch.svelte';
	import Logo from '$lib/components/Logo.svelte';
	import ServerStatus from '$lib/components/ServerStatus.svelte';
	import ThemeSwitch from '$lib/components/ThemeSwitch.svelte';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { isAuditor, loadSession, session, signOut } from '$lib/session.svelte';
	import SessionStatus from '$lib/session/SessionStatus.svelte';
	import SessionTabs from '$lib/session/SessionTabs.svelte';
	import SessionView from '$lib/session/SessionView.svelte';
	import { tabs } from '$lib/session/tabs.svelte';
	import { unlocked } from '$lib/vault/unlocked.svelte';

	let { children } = $props();

	const signInPage = $derived(page.url.pathname.startsWith(resolve('/sign-in')));
	// The devices page shows the active session in place of the device's
	// details (#85).
	const showing = $derived(
		page.url.pathname === resolve('/')
			? tabs.list.find((tab) => tab.key === tabs.active)
			: undefined
	);
	// The devices page and sessions fill the window and scroll inside; the
	// other pages sit in a column and scroll as a whole.
	const fullBleed = $derived(
		page.url.pathname === resolve('/') || page.url.pathname.startsWith(`${resolve('/connect')}/`)
	);

	const links = $derived([
		{ href: resolve('/'), label: m.nav_devices },
		{ href: resolve('/vault'), label: m.nav_vault },
		{ href: resolve('/requests'), label: m.nav_requests },
		...(session.user?.admin
			? [
					{ href: resolve('/users'), label: m.nav_users },
					{ href: resolve('/connectors'), label: m.nav_connectors }
				]
			: []),
		...(isAuditor(session.user) ? [{ href: resolve('/audit'), label: m.nav_audit }] : []),
		...(session.user?.admin ? [{ href: resolve('/settings'), label: m.nav_settings }] : [])
	]);

	/** Up to two letters for the avatar: "Alice Admin" → "AA". */
	const initials = (name: string) =>
		name
			.split(/\s+/)
			.filter(Boolean)
			.slice(0, 2)
			.map((word) => word[0].toUpperCase())
			.join('');

	$effect(() => {
		document.documentElement.lang = getLocale();
		loadSession();
	});

	// Everything but the sign-in page needs a session.
	$effect(() => {
		if (!session.loaded || session.user) return;
		// Signed out elsewhere or expired: the sessions are gone with the page
		// below, and must not start again on the next sign-in; the next user
		// does not find the vault open either.
		tabs.clear();
		unlocked.key = null;
		if (!signInPage) goto(resolve('/sign-in'));
	});

	// A reload or a closed tab ends every session: the browser asks first.
	function beforeUnload(event: BeforeUnloadEvent) {
		if (tabs.list.length > 0) event.preventDefault();
	}

	async function leave() {
		tabs.clear();
		unlocked.key = null;
		await signOut();
		await goto(resolve('/sign-in'));
	}
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>{showing ? `${showing.device.name} · remotehub` : 'remotehub'}</title>
</svelte:head>

<svelte:window onbeforeunload={beforeUnload} />

{#if signInPage}
	<div class="fixed top-3 right-3 flex items-center gap-2">
		<LocaleSwitch />
		<ThemeSwitch />
	</div>
	{@render children()}
{:else if session.user}
	<a
		href="#main"
		class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-20 focus:rounded-md focus:bg-surface focus:px-3 focus:py-2"
	>
		{m.skip_to_content()}
	</a>

	<div class="flex flex-col {fullBleed ? 'h-dvh' : 'min-h-dvh'}">
		<header class="sticky top-0 z-10 border-b border-line bg-sunken/90 backdrop-blur">
			<div class="flex h-15 items-center gap-7 px-4 sm:px-6">
				<a href={resolve('/')} class="flex items-center gap-2.5">
					<Logo />
					<span class="font-display text-lg font-bold tracking-tight">remotehub</span>
				</a>
				<nav class="flex items-center gap-1 text-sm text-ink-2" aria-label={m.nav_label()}>
					{#each links as link (link.href)}
						<a
							href={link.href}
							class="rounded-lg px-3 py-2 hover:bg-surface-2 hover:text-ink aria-[current=page]:bg-surface-2 aria-[current=page]:font-medium aria-[current=page]:text-ink"
							aria-current={page.url.pathname === link.href ? 'page' : undefined}
						>
							{link.label()}
						</a>
					{/each}
				</nav>
				<div class="ml-auto flex items-center gap-2">
					<a
						href={resolve('/account')}
						class="hidden h-9 items-center gap-2.5 rounded-full border border-line bg-surface py-1 pr-3.5 pl-1 text-sm hover:bg-surface-2 md:inline-flex"
						title={m.signed_in_as({ name: session.user.username })}
					>
						<span
							class="flex size-7 items-center justify-center rounded-full bg-surface-2 text-xs font-semibold"
							aria-hidden="true"
						>
							{initials(session.user.display_name)}
						</span>
						{session.user.display_name}
					</a>
					<button
						type="button"
						class="rounded-lg border border-line bg-surface p-2 text-ink-2 hover:text-ink"
						title={m.sign_out()}
						onclick={leave}
					>
						<LogOut size={15} aria-hidden="true" />
						<span class="sr-only">{m.sign_out()}</span>
					</button>
					<LocaleSwitch />
					<ThemeSwitch />
				</div>
			</div>
		</header>

		{#if session.user.kind === 'break_glass'}
			<div
				class="flex items-center justify-center gap-2 bg-critical px-4 py-2 text-sm font-medium text-white"
				role="alert"
			>
				<Siren size={16} aria-hidden="true" />
				{m.break_glass_banner()}
			</div>
		{/if}

		{#if tabs.list.length > 0}
			<SessionTabs />
		{/if}

		<div class="flex flex-1 {fullBleed ? 'min-h-0' : ''}">
			{#if fullBleed}
				<main id="main" class="flex min-h-0 flex-col {showing ? 'flex-none' : 'min-w-0 flex-1'}">
					{@render children()}
				</main>
			{:else}
				<main id="main" class="mx-auto w-full max-w-7xl flex-1 px-4 pt-10 pb-16 sm:px-8">
					{@render children()}
				</main>
			{/if}

			<!-- The sessions stay here on every page, so they outlive a visit to
			     another one; hidden, they keep running. -->
			<div class="relative min-w-0 flex-1 {showing ? '' : 'hidden'}">
				{#each tabs.list as tab (tab.key)}
					<div class="absolute inset-0 overflow-auto {tab.key === showing?.key ? '' : 'hidden'}">
						<SessionView
							device={tab.device}
							askPurpose={tab.askPurpose}
							visible={tab.key === showing?.key}
							onphase={(phase) => tabs.setPhase(tab.key, phase)}
						/>
					</div>
				{/each}
			</div>
		</div>

		<footer class="border-t border-line bg-sunken">
			<div class="flex items-center gap-4 px-4 py-2.5 sm:px-6">
				<div class="shrink-0"><ServerStatus /></div>
				<SessionStatus />
			</div>
		</footer>
	</div>
{/if}
