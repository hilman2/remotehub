<script lang="ts">
	/**
	 * How to start a site connector with Docker or on a Windows server, with
	 * this remotehub's address and release filled in. Right after creating
	 * one (#171) with its token, shown this once; without a token, for a
	 * connector that exists (#186), with the way to update it as well.
	 */
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import Download from '@lucide/svelte/icons/download';
	import { fetchServerState } from '$lib/api/health';
	import { m } from '$lib/paraglide/messages';
	import {
		SERVED,
		dockerCommands,
		dockerUpdate,
		windowsCommands,
		windowsDownloads,
		windowsUpdate
	} from './setup';

	let { token = null, origin }: { token?: string | null; origin: string } = $props();

	type Way = 'docker' | 'windows';
	let way = $state<Way>('docker');
	let version = $state<string | null>(null);
	/** Whether remotehub serves the connector for Windows itself (#188). */
	let served = $state(false);
	/** Which block was copied last, for its button's check mark. */
	let copied = $state<string | null>(null);

	$effect(() => {
		fetchServerState().then((server) => {
			if (server.kind !== 'unreachable') version = server.version;
		});
		// The hash line is small and names the program only where it is served.
		fetch(SERVED.sums)
			.then(async (response) =>
				response.ok ? (await response.text()).includes('remotehub-connector.exe') : false
			)
			.catch(() => false)
			.then((found) => (served = found));
	});

	async function copy(key: string, text: string) {
		try {
			await navigator.clipboard.writeText(text);
			copied = key;
			setTimeout(() => {
				if (copied === key) copied = null;
			}, 2000);
		} catch {
			copied = null;
		}
	}

	const tab = 'rounded-md px-3 py-1 text-sm aria-pressed:bg-surface aria-pressed:shadow-sm';
	const block =
		'overflow-x-auto rounded-lg border border-line bg-page p-3 pr-10 font-mono text-xs whitespace-pre select-all';
	const copyButton =
		'absolute top-2 right-2 rounded-md p-1 text-ink-2 hover:bg-surface-2 hover:text-ink';
	const link =
		'inline-flex items-center gap-1.5 rounded-lg border border-line px-3 py-1.5 text-sm hover:bg-surface-2';
</script>

{#snippet copyable(key: string, label: string, text: string)}
	<div class="relative">
		<pre class={block} aria-label={label}>{text}</pre>
		<button
			type="button"
			class={copyButton}
			aria-label={copied === key ? m.connectors_copied() : m.connectors_copy()}
			title={copied === key ? m.connectors_copied() : m.connectors_copy()}
			onclick={() => copy(key, text)}
		>
			{#if copied === key}
				<Check size={14} aria-hidden="true" />
			{:else}
				<Copy size={14} aria-hidden="true" />
			{/if}
		</button>
	</div>
{/snippet}

{#if token}
	<p class="text-sm">{m.connectors_token_hint()}</p>
	<div class="mt-2">{@render copyable('token', m.connectors_token_label(), token)}</div>
{:else}
	<p class="text-sm">{m.connectors_download_hint()}</p>
{/if}

<h3 class="mt-5 text-sm font-semibold">{m.connectors_setup_title()}</h3>
<div class="mt-2 inline-flex rounded-lg bg-surface-2 p-0.5" role="group">
	<button
		type="button"
		class={tab}
		aria-pressed={way === 'docker'}
		onclick={() => (way = 'docker')}
	>
		{m.connectors_setup_docker()}
	</button>
	<button
		type="button"
		class={tab}
		aria-pressed={way === 'windows'}
		onclick={() => (way = 'windows')}
	>
		{m.connectors_setup_windows()}
	</button>
</div>

{#if version}
	{#if way === 'docker'}
		<p class="mt-3 text-sm text-ink-2">{m.connectors_setup_docker_hint()}</p>
		<div class="mt-2">
			{@render copyable(
				'docker',
				m.connectors_setup_docker_commands(),
				dockerCommands(origin, version)
			)}
		</div>
		{#if !token}
			<h3 class="mt-5 text-sm font-semibold">{m.connectors_update_title()}</h3>
			<p class="mt-2 text-sm text-ink-2">{m.connectors_update_docker_hint()}</p>
			<div class="mt-2">
				{@render copyable(
					'docker-update',
					m.connectors_update_docker_commands(),
					dockerUpdate(origin, version)
				)}
			</div>
		{/if}
	{:else}
		{@const downloads = windowsDownloads(version, served ? origin : null)}
		<div class="mt-3 flex flex-wrap gap-2">
			<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a file, from remotehub or GitHub -->
			<a class={link} href={downloads.program}>
				<Download size={14} aria-hidden="true" />
				{m.connectors_setup_windows_program({ version })}
			</a>
			<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a file, from remotehub or GitHub -->
			<a class={link} href={downloads.sums}>
				<Download size={14} aria-hidden="true" />
				{m.connectors_setup_windows_sums()}
			</a>
		</div>
		<p class="mt-3 text-sm text-ink-2">
			{served ? m.connectors_setup_windows_served_hint() : m.connectors_setup_windows_hint()}
		</p>
		<div class="mt-2">
			{@render copyable(
				'windows',
				m.connectors_setup_windows_commands(),
				windowsCommands(origin, served)
			)}
		</div>
		{#if !token}
			<h3 class="mt-5 text-sm font-semibold">{m.connectors_update_title()}</h3>
			<p class="mt-2 text-sm text-ink-2">
				{served ? m.connectors_update_windows_served_hint() : m.connectors_update_windows_hint()}
			</p>
			<div class="mt-2">
				{@render copyable(
					'windows-update',
					m.connectors_update_windows_commands(),
					windowsUpdate(origin, served)
				)}
			</div>
		{/if}
	{/if}
{/if}

{#if token}
	<p class="mt-4 text-sm">{m.connectors_token_closed()}</p>
{/if}
<p class="mt-2 text-xs text-ink-3">{m.connectors_token_docs()}</p>
