<script lang="ts">
	/**
	 * Connects the browser extension (#201, ADR 0017). The extension opens
	 * this page with `chrome.identity.launchWebAuthFlow`, signed in or not;
	 * the layout sends the user through sign-in and back. After the user
	 * allows it, the page fetches a one-time code and hands it to the
	 * extension through the address Chromium reserves for it, which only that
	 * extension receives.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Puzzle from '@lucide/svelte/icons/puzzle';
	import { page } from '$app/state';
	import { errorMessage } from '$lib/api/errors';
	import Logo from '$lib/components/Logo.svelte';
	import { answerUrl, askCode, browserName, connectRequest } from '$lib/extension/connect';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';

	const request = $derived(connectRequest(page.url.searchParams));
	let busy = $state(false);
	let error = $state<string | null>(null);
	let answered = $state(false);

	async function allow() {
		if (!request) return;
		busy = true;
		error = null;
		const agent = (navigator as { userAgentData?: Parameters<typeof browserName>[0] })
			.userAgentData;
		const result = await askCode(request, browserName(agent, navigator.userAgent));
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		answered = true;
		location.assign(answerUrl(request, { code: result.data.code, locale: getLocale() }));
	}

	function deny() {
		if (!request) return;
		answered = true;
		location.assign(answerUrl(request, { error: 'denied' }));
	}

	const button =
		'inline-flex items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-medium disabled:opacity-60';
</script>

<main class="grid min-h-dvh place-items-center bg-page p-6 text-ink">
	<section
		class="flex w-full max-w-md flex-col gap-5 rounded-card border border-line bg-surface p-8 shadow-card"
		aria-labelledby="connect-title"
	>
		<div class="flex items-center gap-2.5">
			<Logo />
			<span class="font-display text-lg font-bold tracking-tight">remotehub</span>
		</div>

		{#if !request}
			<p class="flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={18} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{m.extension_connect_invalid()}
			</p>
		{:else if answered}
			<p class="flex items-start gap-2 text-sm" role="status">
				<CircleCheck size={18} class="mt-0.5 shrink-0 text-ok" aria-hidden="true" />
				{m.extension_connect_answered()}
			</p>
		{:else}
			<h1 id="connect-title" class="flex items-center gap-2 font-display text-2xl font-semibold">
				<Puzzle size={22} aria-hidden="true" />
				{m.extension_connect_title()}
			</h1>
			<p class="text-sm text-ink-2">
				{m.extension_connect_hint({ user: session.user?.display_name ?? '' })}
			</p>
			<ul class="flex list-disc flex-col gap-1.5 pl-5 text-sm">
				<li>{m.extension_connect_lists()}</li>
				<li>{m.extension_connect_fills()}</li>
				<li>{m.extension_connect_audited()}</li>
			</ul>
			<p class="text-xs text-ink-3">
				{m.extension_connect_id()}
				<span class="font-mono break-all">{request.extensionId}</span>
			</p>
			{#if error}
				<p class="flex items-start gap-2 text-sm" role="alert">
					<CircleAlert size={18} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
					{error}
				</p>
			{/if}
			<div class="flex flex-wrap gap-3">
				<button
					type="button"
					class="{button} bg-accent text-accent-ink hover:opacity-90"
					disabled={busy}
					onclick={allow}
				>
					{m.extension_connect_allow()}
				</button>
				<button
					type="button"
					class="{button} border border-line-strong bg-surface hover:bg-surface-2"
					disabled={busy}
					onclick={deny}
				>
					{m.extension_connect_deny()}
				</button>
			</div>
		{/if}
	</section>
</main>
