<script lang="ts">
	/**
	 * A shared credential's files and earlier versions (#100). Files are
	 * added and removed with `edit`; downloading them and looking at earlier
	 * passwords takes `reveal`, and the server records each time.
	 */
	import Upload from '@lucide/svelte/icons/upload';
	import History from '@lucide/svelte/icons/history';
	import Paperclip from '@lucide/svelte/icons/paperclip';
	import X from '@lucide/svelte/icons/x';
	import { allows, type Credential } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import {
		attachmentUrl,
		deleteAttachment,
		loadVersions,
		reveal,
		uploadAttachment,
		type Version
	} from '$lib/api/reveal';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let { credential, onchange }: { credential: Credential; onchange: () => void } = $props();

	const MAX_FILE = 5 * 1024 * 1024;

	let versions = $state<Version[] | null>(null);
	/** Earlier passwords shown, by version. */
	let earlier = $state<Record<number, string>>({});
	let error = $state<string | null>(null);

	const when = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
	const kilobytes = new Intl.NumberFormat(formatLocale(), { style: 'unit', unit: 'kilobyte' });

	async function add(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		if (file.size > MAX_FILE) {
			error = m.vault_file_too_large({ megabytes: MAX_FILE / 1024 / 1024 });
			return;
		}
		const result = await uploadAttachment(credential.id, file);
		error = result.ok ? null : errorMessage(result.code);
		onchange();
	}

	async function remove(attachment: string) {
		const result = await deleteAttachment(credential.id, attachment);
		error = result.ok ? null : errorMessage(result.code);
		onchange();
	}

	async function showHistory() {
		const result = await loadVersions(credential.id);
		if (result.ok) versions = result.data.filter((v) => v.version !== credential.version);
		else error = errorMessage(result.code);
	}

	async function showVersion(version: number) {
		const result = await reveal('credentials', credential.id, 'show', version);
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		earlier = { ...earlier, [version]: result.data.password ?? result.data.private_key ?? '' };
	}

	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line-strong px-2.5 py-1 text-sm hover:bg-surface-2';
</script>

<div class="flex flex-col gap-3 text-sm">
	{#if credential.attachments.length > 0 || allows(credential.role, 'edit')}
		<div>
			<h4 class="eyebrow">{m.vault_files()}</h4>
			<ul class="mt-1 space-y-1">
				{#each credential.attachments as file (file.id)}
					<li class="flex items-center gap-2">
						<Paperclip size={14} class="text-ink-3" aria-hidden="true" />
						{#if allows(credential.role, 'reveal')}
							<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a download -->
							<a class="text-accent hover:underline" href={attachmentUrl(credential.id, file.id)}
								>{file.name}</a
							>
						{:else}
							<span>{file.name}</span>
						{/if}
						<span class="text-xs text-ink-3"
							>{kilobytes.format(Math.max(1, Math.round(file.size / 1024)))}</span
						>
						{#if allows(credential.role, 'edit')}
							<button
								type="button"
								class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-critical"
								title={m.vault_field_remove({ name: file.name })}
								onclick={() => remove(file.id)}
							>
								<X size={14} aria-hidden="true" />
								<span class="sr-only">{m.vault_field_remove({ name: file.name })}</span>
							</button>
						{/if}
					</li>
				{/each}
			</ul>
			{#if allows(credential.role, 'edit')}
				<label class="{button} mt-2 cursor-pointer">
					<Upload size={14} aria-hidden="true" />
					{m.vault_add_file()}
					<input type="file" class="sr-only" onchange={add} />
				</label>
			{/if}
		</div>
	{/if}

	{#if allows(credential.role, 'reveal') && credential.version > 1}
		<div>
			{#if versions === null}
				<button type="button" class={button} onclick={showHistory}>
					<History size={14} aria-hidden="true" />
					{m.vault_history()}
				</button>
			{:else}
				<h4 class="eyebrow">{m.vault_history()}</h4>
				<ul class="mt-1 space-y-1">
					{#each versions as version (version.version)}
						<li class="flex flex-wrap items-center gap-2">
							<span class="tabular-nums"
								>{m.credential_version({ version: version.version })} · {when.format(
									new Date(version.created_at)
								)}</span
							>
							{#if earlier[version.version] !== undefined}
								<span class="font-mono break-all select-all" data-testid="earlier-password"
									>{earlier[version.version]}</span
								>
							{:else}
								<button
									type="button"
									class="rounded-md px-2 py-0.5 text-xs text-ink-2 hover:bg-surface-2"
									onclick={() => showVersion(version.version)}
								>
									{m.reveal_show()}
								</button>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	{/if}
	{#if error}
		<p class="text-critical" role="alert">{error}</p>
	{/if}
</div>
