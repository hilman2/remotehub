<script lang="ts">
	/**
	 * An entry of the personal vault (#190): opened in this browser, so its
	 * password, codes, files and earlier states are all here to show.
	 */
	import Copy from '@lucide/svelte/icons/copy';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import Pencil from '@lucide/svelte/icons/pencil';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { icon } from './icons';
	import TotpCode from './TotpCode.svelte';
	import { totpOf } from './totp';
	import type { EntryContent, FileRef } from './vault';

	let {
		content,
		where,
		onedit,
		onused,
		ondownload
	}: {
		content: EntryContent;
		/** The personal folder's path; empty at the top. */
		where: string;
		onedit: () => void;
		/** When the password was shown or copied, for sorting by use. */
		onused: () => void;
		ondownload: (file: FileRef) => void;
	} = $props();

	let revealed = $state(false);
	let copied = $state<'username' | 'password' | null>(null);

	const Icon = $derived(icon(content.icon));
	/** Only web addresses become links; anything else stays text. */
	const link = $derived(/^https?:\/\//i.test(content.url) ? content.url : null);

	async function copy(what: 'username' | 'password') {
		if (what === 'password') onused();
		await navigator.clipboard.writeText(what === 'password' ? content.password : content.username);
		copied = what;
		setTimeout(() => (copied = null), 2000);
	}

	function toggle() {
		if (!revealed) onused();
		revealed = !revealed;
	}

	const when = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
	const kilobytes = new Intl.NumberFormat(formatLocale(), { style: 'unit', unit: 'kilobyte' });
	const size = (bytes: number) => kilobytes.format(Math.max(1, Math.round(bytes / 1024)));

	const small =
		'inline-flex h-8 items-center gap-1.5 rounded-lg border border-line px-2.5 text-sm hover:bg-surface-2';
	const row = 'grid grid-cols-[8rem_minmax(0,1fr)_auto] items-center gap-3 py-2.5';
</script>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-4">
		<span
			class="flex size-12 shrink-0 items-center justify-center rounded-xl bg-surface-2 text-warning"
			aria-hidden="true"
		>
			<Icon size={22} />
		</span>
		<div class="min-w-0 flex-1">
			<h2 class="text-2xl font-semibold break-words">{content.title}</h2>
			<p class="text-sm text-ink-3">
				{where ? m.vault_in_personal_folder({ path: where }) : m.vault_personal_only()}
			</p>
		</div>
		<button
			type="button"
			class="inline-flex h-9 items-center gap-2 rounded-lg border border-line-strong bg-surface px-3 text-sm hover:bg-surface-2"
			onclick={onedit}
		>
			<Pencil size={16} aria-hidden="true" />
			{m.catalog_edit()}
		</button>
	</div>

	<dl class="divide-y divide-line rounded-card border border-line bg-surface px-4 text-sm">
		<div class={row}>
			<dt class="text-ink-2">{m.field_username()}</dt>
			<dd class="truncate font-mono">{content.username}</dd>
			<dd>
				{#if content.username}
					<button type="button" class={small} onclick={() => copy('username')}>
						<Copy size={14} aria-hidden="true" />
						{copied === 'username' ? m.vault_copied() : m.vault_copy()}
					</button>
				{/if}
			</dd>
		</div>
		<div class={row}>
			<dt class="text-ink-2">{m.field_password()}</dt>
			<dd class="font-mono break-all">
				{#if revealed}{content.password}{:else}<span aria-hidden="true">••••••••••••</span><span
						class="sr-only">{m.credential_hidden()}</span
					>{/if}
			</dd>
			<dd class="flex gap-2">
				<button type="button" class={small} onclick={toggle}>
					{#if revealed}
						<EyeOff size={14} aria-hidden="true" />
						{m.vault_hide()}
					{:else}
						<Eye size={14} aria-hidden="true" />
						{m.vault_show()}
					{/if}
				</button>
				<button type="button" class={small} onclick={() => copy('password')}>
					<Copy size={14} aria-hidden="true" />
					{copied === 'password' ? m.vault_copied() : m.vault_copy()}
				</button>
			</dd>
		</div>
		{#if content.url}
			<div class={row}>
				<dt class="text-ink-2">{m.field_url()}</dt>
				<dd class="col-span-2 break-all">
					{#if link}
						<!-- eslint-disable svelte/no-navigation-without-resolve -- another site -->
						<a
							class="text-accent hover:underline"
							href={link}
							target="_blank"
							rel="noreferrer noopener">{content.url}</a
						>
						<!-- eslint-enable svelte/no-navigation-without-resolve -->
					{:else}
						{content.url}
					{/if}
				</dd>
			</div>
		{/if}
		{#each content.fields ?? [] as field (field.name)}
			{@const totp = totpOf(field.name, field.value)}
			<div class={row}>
				<dt class="truncate text-ink-2">{field.name}</dt>
				<dd class="col-span-2 break-all">
					{#if totp && revealed}
						<TotpCode params={totp} />
					{:else if (field.protected || totp) && !revealed}
						<span aria-hidden="true">••••••</span>
						<span class="sr-only">{m.credential_hidden()}</span>
					{:else}
						<span class="font-mono">{field.value}</span>
					{/if}
				</dd>
			</div>
		{/each}
	</dl>

	{#if content.notes}
		<section>
			<h3 class="eyebrow">{m.field_notes()}</h3>
			<p class="mt-1 text-sm whitespace-pre-line">{content.notes}</p>
		</section>
	{/if}

	{#if (content.attachments ?? []).length > 0}
		<section>
			<h3 class="eyebrow">{m.vault_files()}</h3>
			<ul class="mt-1 flex flex-col gap-1 text-sm">
				{#each content.attachments ?? [] as file (file.id)}
					<li>
						<button
							type="button"
							class="text-accent hover:underline"
							onclick={() => ondownload(file)}
						>
							{file.name} ({size(file.size)})
						</button>
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if revealed && content.history?.length}
		<details class="text-sm text-ink-2">
			<summary class="cursor-pointer">{m.vault_history()}</summary>
			<ul class="mt-1 space-y-1">
				{#each content.history as earlier (earlier.at)}
					<li>
						<span class="tabular-nums">{when.format(new Date(earlier.at))}</span>:
						<span class="font-mono break-all" data-testid="earlier-password"
							>{earlier.password}</span
						>
					</li>
				{/each}
			</ul>
		</details>
	{/if}
</div>
