<script lang="ts">
	/**
	 * The browser extension on *My account* (#201): where to get it, how to
	 * install it by hand or by policy, and the extensions connected to this
	 * account, each of which can be ended here.
	 */
	import Download from '@lucide/svelte/icons/download';
	import Puzzle from '@lucide/svelte/icons/puzzle';
	import { problemMessage } from '$lib/api/errors';
	import {
		EXTENSION_DOWNLOADS,
		endExtension,
		loadExtensions,
		servedExtension,
		servedExtensionId,
		type ConnectedExtension
	} from '$lib/extension/connect';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';

	let connected = $state<ConnectedExtension[]>([]);
	let served = $state({ zip: false, crx: false });
	let id = $state<string | null>(null);
	let busy = $state(false);
	let error = $state<string | null>(null);

	const origin = location.origin;
	const policy = $derived(id ? `${id};${origin}${EXTENSION_DOWNLOADS.updates}` : null);

	async function load() {
		const result = await loadExtensions();
		if (result.ok) connected = result.data;
		else error = problemMessage(result);
	}

	$effect(() => {
		load();
		servedExtension().then(async (found) => {
			served = found;
			if (found.crx && session.user?.admin) id = await servedExtensionId();
		});
	});

	async function end(extension: ConnectedExtension) {
		busy = true;
		const result = await endExtension(extension.id);
		busy = false;
		error = result.ok ? null : problemMessage(result);
		await load();
	}

	const when = (at: string) =>
		new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'medium', timeStyle: 'short' }).format(
			new Date(at)
		);

	const button =
		'inline-flex items-center gap-2 self-start rounded-xl border border-line-strong bg-surface px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-60';
</script>

<section
	class="flex max-w-3xl flex-col gap-3 rounded-card border border-line bg-surface p-6"
	aria-labelledby="account-extension"
>
	<h2 id="account-extension" class="flex items-center gap-2 text-lg font-semibold">
		<Puzzle size={18} aria-hidden="true" />
		{m.account_extension()}
	</h2>
	<p class="text-sm text-ink-2">{m.account_extension_hint()}</p>

	{#if served.zip}
		<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a file remotehub serves -->
		<a class={button} href={EXTENSION_DOWNLOADS.zip} download>
			<Download size={16} aria-hidden="true" />
			{m.account_extension_download()}
		</a>
		<ol class="flex list-decimal flex-col gap-1 pl-5 text-sm">
			<li>{m.account_extension_step_unzip()}</li>
			<li>{m.account_extension_step_developer()}</li>
			<li>{m.account_extension_step_load()}</li>
			<li>
				{m.account_extension_step_connect()}
				<span class="font-mono">{origin}</span>
			</li>
		</ol>
		<p class="text-sm text-ink-2">{m.account_extension_updates()}</p>
	{:else}
		<p class="text-sm text-ink-2">{m.account_extension_not_served()}</p>
	{/if}

	{#if policy}
		<h3 class="mt-2 text-sm font-semibold">{m.account_extension_policy()}</h3>
		<p class="text-sm text-ink-2">{m.account_extension_policy_hint()}</p>
		<pre
			class="rounded-lg border border-line bg-page p-3 font-mono text-xs break-all whitespace-pre-wrap select-all"
			data-testid="extension-policy">{policy}</pre>
	{/if}

	<h3 class="mt-2 text-sm font-semibold">{m.account_extension_connected()}</h3>
	{#if connected.length === 0}
		<p class="text-sm text-ink-2">{m.account_extension_none()}</p>
	{:else}
		<ul class="flex flex-col gap-2">
			{#each connected as extension (extension.id)}
				<li class="flex flex-wrap items-center gap-3 text-sm">
					<span class="font-medium">{extension.name}</span>
					<span class="text-ink-2">
						{m.account_extension_since({
							since: when(extension.created_at),
							seen: when(extension.last_seen_at)
						})}
					</span>
					<button type="button" class={button} disabled={busy} onclick={() => end(extension)}>
						{m.account_extension_end({ name: extension.name })}
					</button>
				</li>
			{/each}
		</ul>
	{/if}
	{#if error}
		<p class="text-sm text-critical" role="alert">{error}</p>
	{/if}
</section>
