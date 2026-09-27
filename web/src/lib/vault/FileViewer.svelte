<script lang="ts">
	/**
	 * A vault file shown in the page (#200), so it need not lie unencrypted
	 * in the downloads. `preview` decides what may be shown; a download stays
	 * possible, on purpose.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Download from '@lucide/svelte/icons/download';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { TEXT_LIMIT, preview, type Preview } from './preview';

	let {
		name,
		load,
		ondownload
	}: {
		name: string;
		/** The file's content; null when it cannot be had. */
		load: () => Promise<Blob | null>;
		ondownload: () => void;
	} = $props();

	let shown = $state<Preview | 'loading' | 'failed'>('loading');
	/** Where an image or a PDF is shown from, made from the checked bytes. */
	let url = $state<string | null>(null);

	$effect(() => {
		let gone = false;
		let made: string | null = null;
		load().then(async (blob) => {
			if (gone) return;
			if (!blob) {
				shown = 'failed';
				return;
			}
			const bytes = new Uint8Array(await blob.arrayBuffer());
			// Closed meanwhile: an address made now would never be revoked.
			if (gone) return;
			const seen = preview(name, bytes);
			if (seen.kind === 'image' || seen.kind === 'pdf') {
				// The type is ours, from the signature: never one the server or the file claims.
				const type = seen.kind === 'pdf' ? 'application/pdf' : seen.type;
				made = URL.createObjectURL(new Blob([bytes], { type }));
				url = made;
			}
			shown = seen;
		});
		return () => {
			gone = true;
			if (made) URL.revokeObjectURL(made);
		};
	});

	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
</script>

<div class="min-h-40">
	{#if shown === 'loading'}
		<p class="text-sm text-ink-2" role="status">{m.viewer_loading()}</p>
	{:else if shown === 'failed'}
		<p class="flex items-center gap-2 text-sm text-critical" role="alert">
			<CircleAlert size={16} aria-hidden="true" />
			{m.viewer_failed()}
		</p>
	{:else if shown.kind === 'text'}
		<pre
			class="max-h-[70vh] overflow-auto rounded-lg border border-line bg-page p-3 font-mono text-xs break-words whitespace-pre-wrap"
			data-testid="file-text">{shown.text}</pre>
		{#if shown.truncated}
			<p class="mt-2 text-xs text-ink-3">
				{m.viewer_truncated({ count: new Intl.NumberFormat(formatLocale()).format(TEXT_LIMIT) })}
			</p>
		{/if}
	{:else if shown.kind === 'image' && url}
		<img
			src={url}
			alt={name}
			class="mx-auto max-h-[70vh] max-w-full rounded-lg border border-line bg-page object-contain"
			data-testid="file-image"
		/>
	{:else if shown.kind === 'pdf' && url}
		<iframe
			src={url}
			title={name}
			class="h-[75vh] w-full rounded-lg border border-line bg-page"
			data-testid="file-pdf"
		></iframe>
	{:else}
		<p class="text-sm text-ink-2">{m.viewer_none()}</p>
	{/if}
</div>

<div class="mt-4 flex flex-wrap items-center gap-3">
	<button type="button" class={button} onclick={ondownload}>
		<Download size={15} aria-hidden="true" />
		{m.viewer_download()}
	</button>
	<span class="text-xs text-ink-3">{m.viewer_download_hint()}</span>
</div>
