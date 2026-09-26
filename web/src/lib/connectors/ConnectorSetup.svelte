<script lang="ts">
	/**
	 * Shown once after creating a site connector (#171): its token, and how to
	 * start it with Docker or on a Windows server, with this remotehub's
	 * address and release filled in.
	 */
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import Download from '@lucide/svelte/icons/download';
	import { fetchServerState } from '$lib/api/health';
	import { m } from '$lib/paraglide/messages';
	import { dockerCommands, windowsCommands, windowsDownloads } from './setup';

	let { token, origin }: { token: string; origin: string } = $props();

	type Way = 'docker' | 'windows';
	let way = $state<Way>('docker');
	let version = $state<string | null>(null);
	/** Which block was copied last, for its button's check mark. */
	let copied = $state<string | null>(null);

	$effect(() => {
		fetchServerState().then((server) => {
			if (server.kind !== 'unreachable') version = server.version;
		});
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

<p class="text-sm">{m.connectors_token_hint()}</p>
<div class="mt-2">{@render copyable('token', m.connectors_token_label(), token)}</div>

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
	{:else}
		{@const downloads = windowsDownloads(version)}
		<div class="mt-3 flex flex-wrap gap-2">
			<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a download from GitHub -->
			<a class={link} href={downloads.program}>
				<Download size={14} aria-hidden="true" />
				{m.connectors_setup_windows_program({ version })}
			</a>
			<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a download from GitHub -->
			<a class={link} href={downloads.sums}>
				<Download size={14} aria-hidden="true" />
				{m.connectors_setup_windows_sums()}
			</a>
		</div>
		<p class="mt-3 text-sm text-ink-2">{m.connectors_setup_windows_hint()}</p>
		<div class="mt-2">
			{@render copyable('windows', m.connectors_setup_windows_commands(), windowsCommands(origin))}
		</div>
	{/if}
{/if}

<p class="mt-4 text-sm">{m.connectors_token_closed()}</p>
<p class="mt-2 text-xs text-ink-3">{m.connectors_token_docs()}</p>
