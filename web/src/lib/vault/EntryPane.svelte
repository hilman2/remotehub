<script lang="ts">
	/**
	 * The chosen entry below the table (#193), as KeePass shows it: where it
	 * lies, its fields with copy buttons, its one-time code on request, files,
	 * tags, expiry and who may use it. Shared secrets come from the server
	 * and each look is audited; personal ones are open in this browser.
	 */
	import Clock from '@lucide/svelte/icons/clock';
	import Copy from '@lucide/svelte/icons/copy';
	import Download from '@lucide/svelte/icons/download';
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import Lock from '@lucide/svelte/icons/lock';
	import Pencil from '@lucide/svelte/icons/pencil';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { allows } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { requestableRoles } from '$lib/api/requests';
	import { attachmentUrl, credentialCode, reveal, viewAttachment } from '$lib/api/reveal';
	import { ROLE_LABELS } from '$lib/catalog/labels';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import FileViewer from './FileViewer.svelte';
	import { icon } from './icons';
	import { isExpired, isExpiring, personalTotp, today, type Item } from './items';
	import TotpCode from './TotpCode.svelte';
	import { totpOf } from './totp';
	import type { FileRef } from './vault';

	let {
		item,
		oncopy,
		onedit,
		onrestore,
		onrequest,
		onread,
		ondownload
	}: {
		item: Item;
		oncopy: (item: Item, what: 'username' | 'password' | 'totp') => void;
		onedit: () => void;
		onrestore: () => void;
		onrequest: () => void;
		/** A personal entry's file, opened in this browser; null if it cannot be. */
		onread: (file: FileRef) => Promise<Blob | null>;
		/** A personal entry's file, opened in this browser and downloaded. */
		ondownload: (file: FileRef) => void;
	} = $props();

	/** The file shown in the viewer (#200). */
	let viewing = $state<{
		name: string;
		load: () => Promise<Blob | null>;
		download: () => void;
	} | null>(null);
	let viewerOpen = $state(false);

	/** A file of the entry as a download: its own choice, audited as such. */
	function download(file: { id: string; name: string }) {
		if (item.source === 'personal') {
			ondownload(file as FileRef);
			return;
		}
		const link = document.createElement('a');
		link.href = attachmentUrl(item.id, file.id);
		link.download = file.name;
		link.click();
	}

	function view(file: { id: string; name: string }) {
		const shown = item;
		viewing = {
			name: file.name,
			load: () =>
				shown.source === 'personal' ? onread(file as FileRef) : viewAttachment(shown.id, file.id),
			download: () => download(file)
		};
		viewerOpen = true;
	}

	/** How long a shown secret stays. */
	const SECONDS = 30;

	let password = $state<string | null>(null);
	/** A shared entry's protected fields, shown with its password. */
	let revealed = $state<Record<string, string>>({});
	/** A shared one-time code and until when it holds (ms). */
	let code = $state<{ code: string; until: number } | null>(null);
	let showCode = $state(false);
	let now = $state(Date.now());
	let error = $state<string | null>(null);

	const may = (role: Parameters<typeof allows>[1]) =>
		item.source === 'personal' || allows(item.credential.role, role);
	const Icon = $derived(icon(item.icon));
	const day = today();
	const personalCode = $derived(
		item.source === 'personal' ? totpOf('otp', personalTotp(item.entry)) : null
	);
	const link = $derived(/^https?:\/\//i.test(item.url) ? item.url : null);
	const dateFormat = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'medium' });
	const timeFormat = new Intl.DateTimeFormat(formatLocale(), {
		dateStyle: 'medium',
		timeStyle: 'short'
	});
	const kilobytes = new Intl.NumberFormat(formatLocale(), { style: 'unit', unit: 'kilobyte' });
	const size = (bytes: number) => kilobytes.format(Math.max(1, Math.round(bytes / 1024)));

	$effect(() => {
		// Another entry forgets what was shown of the last.
		void item.key;
		password = null;
		revealed = {};
		code = null;
		showCode = false;
		error = null;
	});

	$effect(() => {
		if (password === null && !code) return;
		const timer = setInterval(() => {
			now = Date.now();
			if (code && now >= code.until) code = null;
		}, 1000);
		const hide = setTimeout(() => {
			password = null;
			revealed = {};
		}, SECONDS * 1000);
		return () => {
			clearInterval(timer);
			clearTimeout(hide);
		};
	});

	async function togglePassword() {
		if (password !== null) {
			password = null;
			revealed = {};
			return;
		}
		if (item.source === 'personal') {
			password = item.entry.content?.password ?? '';
			return;
		}
		const result = await reveal('credentials', item.id, 'show');
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		password = result.data.password ?? '';
		revealed = Object.fromEntries((result.data.fields ?? []).map((f) => [f.name, f.value]));
	}

	async function toggleCode() {
		if (showCode || code) {
			showCode = false;
			code = null;
			return;
		}
		if (item.source === 'personal') {
			showCode = true;
			return;
		}
		const result = await credentialCode(item.id, 'show');
		if (result.ok) {
			code = { code: result.data.code, until: Date.now() + result.data.remaining * 1000 };
			now = Date.now();
		} else error = errorMessage(result.code);
	}

	const small =
		'inline-flex h-7 items-center gap-1.5 rounded-md border border-line px-2 text-xs hover:bg-surface-2';
	const key = 'py-1.5 text-ink-2';
	const value = 'min-w-0 py-1.5 break-words';
</script>

<section
	aria-label={m.vault_details()}
	class="grid shrink-0 gap-6 border-t border-line-strong bg-sunken px-5 py-4 lg:grid-cols-[minmax(0,1fr)_20rem]"
>
	<div class="min-w-0">
		<div class="flex items-start gap-3">
			<Icon size={20} class="mt-1 shrink-0 text-warning" aria-hidden="true" />
			<div class="min-w-0 flex-1">
				<h2 class="truncate text-lg font-semibold">{item.title}</h2>
				<p class="truncate text-xs text-ink-3">
					{item.source === 'personal'
						? m.vault_in_personal_folder({ path: item.where || m.vault_personal() })
						: m.vault_in_shared_folder({ path: item.where })}
				</p>
			</div>
			{#if item.deleted}
				{#if may('edit')}
					<button type="button" class={small} onclick={onrestore}>
						<RotateCcw size={13} aria-hidden="true" />
						{m.vault_restore()}
					</button>
				{/if}
			{:else}
				{#if item.source === 'shared' && requestableRoles(item.credential.role, 'credential').length > 0}
					<button type="button" class={small} onclick={onrequest}>
						<Clock size={13} aria-hidden="true" />
						{m.request_access()}
					</button>
				{/if}
				{#if may('edit')}
					<button type="button" class={small} onclick={onedit}>
						<Pencil size={13} aria-hidden="true" />
						{m.catalog_edit()}
					</button>
				{/if}
			{/if}
		</div>

		<!-- The values take the width they need, so their buttons follow right after them. -->
		<dl
			class="mt-3 grid grid-cols-[8rem_fit-content(28rem)_minmax(0,1fr)] items-center gap-x-3 text-sm"
		>
			<dt class={key}>{m.field_username()}</dt>
			<dd class="{value} font-mono">{item.username}</dd>
			<dd>
				{#if item.username}
					<button
						type="button"
						class={small}
						aria-label={m.vault_copy_username()}
						onclick={() => oncopy(item, 'username')}
					>
						<Copy size={13} aria-hidden="true" />
						{m.vault_copy()}
					</button>
				{/if}
			</dd>

			<dt class={key}>{m.field_password()}</dt>
			<dd class="{value} font-mono" data-testid={password !== null ? 'revealed' : undefined}>
				{#if password !== null}{password}{:else}<span aria-hidden="true">••••••••••••</span><span
						class="sr-only">{m.credential_hidden()}</span
					>{/if}
			</dd>
			<dd class="flex gap-1.5">
				{#if may('reveal')}
					<button
						type="button"
						class={small}
						aria-label={password !== null ? m.vault_hide_password() : m.vault_show_password()}
						onclick={togglePassword}
					>
						{#if password !== null}
							<EyeOff size={13} aria-hidden="true" />
							{m.vault_hide()}
						{:else}
							<Eye size={13} aria-hidden="true" />
							{m.vault_show()}
						{/if}
					</button>
					<button
						type="button"
						class={small}
						aria-label={m.vault_copy_password()}
						onclick={() => oncopy(item, 'password')}
					>
						<Copy size={13} aria-hidden="true" />
						{m.vault_copy()}
					</button>
				{/if}
			</dd>

			{#if item.hasTotp}
				<dt class={key}>{m.vault_totp()}</dt>
				<dd class={value}>
					{#if showCode && personalCode}
						<TotpCode params={personalCode} />
					{:else if code}
						<span class="inline-flex items-baseline gap-2">
							<span class="font-mono tracking-widest" data-testid="totp-code">{code.code}</span>
							<span class="text-xs text-ink-3">
								{m.vault_totp_left({ seconds: Math.max(0, Math.ceil((code.until - now) / 1000)) })}
							</span>
						</span>
					{:else}
						<span class="text-ink-3" aria-hidden="true">••• •••</span>
					{/if}
				</dd>
				<dd class="flex gap-1.5">
					{#if may('reveal')}
						<button
							type="button"
							class={small}
							aria-label={showCode || code ? m.vault_hide_totp() : m.vault_show_totp()}
							onclick={toggleCode}
						>
							{#if showCode || code}
								<EyeOff size={13} aria-hidden="true" />
								{m.vault_hide()}
							{:else}
								<Eye size={13} aria-hidden="true" />
								{m.vault_show()}
							{/if}
						</button>
						<button
							type="button"
							class={small}
							aria-label={m.vault_copy_totp()}
							onclick={() => oncopy(item, 'totp')}
						>
							<Copy size={13} aria-hidden="true" />
							{m.vault_copy()}
						</button>
					{/if}
				</dd>
			{/if}

			{#if item.url}
				<dt class={key}>{m.field_url()}</dt>
				<dd class={value}>
					{#if link}
						<!-- eslint-disable svelte/no-navigation-without-resolve -- another site -->
						<a
							class="text-accent hover:underline"
							href={link}
							target="_blank"
							rel="noreferrer noopener">{item.url}</a
						>
						<!-- eslint-enable svelte/no-navigation-without-resolve -->
					{:else}
						{item.url}
					{/if}
				</dd>
				<dd>
					{#if link}
						<!-- eslint-disable svelte/no-navigation-without-resolve -- another site -->
						<a class={small} href={link} target="_blank" rel="noreferrer noopener">
							<ExternalLink size={13} aria-hidden="true" />
							{m.vault_open_url()}
						</a>
						<!-- eslint-enable svelte/no-navigation-without-resolve -->
					{/if}
				</dd>
			{/if}

			{#if item.source === 'personal'}
				{#each (item.entry.content?.fields ?? []).filter((f) => !totpOf(f.name, f.value)) as field (field.name)}
					<dt class="{key} truncate">{field.name}</dt>
					<dd class="{value} col-span-2 font-mono">
						{#if field.protected && password === null}
							<span aria-hidden="true">••••••</span>
							<span class="sr-only">{m.credential_hidden()}</span>
						{:else}
							{field.value}
						{/if}
					</dd>
				{/each}
			{:else}
				{#each item.credential.fields as field (field.name)}
					<dt class="{key} truncate">{field.name}</dt>
					<dd class="{value} col-span-2 font-mono">
						{#if field.protected && field.name in revealed}
							<span data-testid="revealed-field">{revealed[field.name]}</span>
						{:else if field.protected}
							<span class="inline-flex items-center gap-1.5 font-sans text-ink-2">
								<Lock size={13} aria-hidden="true" />
								{m.credential_hidden()}
							</span>
						{:else}
							{field.value}
						{/if}
					</dd>
				{/each}
			{/if}

			{#if item.notes}
				<dt class="{key} self-start">{m.field_notes()}</dt>
				<dd class="{value} col-span-2 whitespace-pre-line">{item.notes}</dd>
			{/if}

			{#if item.files > 0}
				<dt class="{key} self-start">{m.vault_files()}</dt>
				<dd class="{value} col-span-2 flex flex-wrap gap-x-4 gap-y-1">
					{#if item.source === 'personal'}
						{#each item.entry.content?.attachments ?? [] as file (file.id)}
							{@render fileItem(file)}
						{/each}
					{:else}
						{#each item.credential.attachments as file (file.id)}
							{#if may('reveal')}
								{@render fileItem(file)}
							{:else}
								<span>{file.name}</span>
							{/if}
						{/each}
					{/if}
				</dd>
			{/if}
		</dl>
		{#if error}
			<p class="mt-2 text-sm text-critical" role="alert">{error}</p>
		{/if}
	</div>

	<div class="flex flex-col gap-3 text-sm text-ink-2">
		{#if item.tags.length > 0}
			<ul class="flex flex-wrap gap-1.5" aria-label={m.vault_tags()}>
				{#each item.tags as tag (tag)}
					<li class="chip">{tag}</li>
				{/each}
			</ul>
		{/if}
		{#if item.expires}
			<p class="flex items-center gap-2 {isExpiring(item, day) ? 'font-medium text-warning' : ''}">
				{#if isExpiring(item, day)}<TriangleAlert size={14} aria-hidden="true" />{/if}
				{isExpired(item, day)
					? m.vault_expired_on({ date: dateFormat.format(new Date(`${item.expires}T00:00:00`)) })
					: m.vault_expires_on({ date: dateFormat.format(new Date(`${item.expires}T00:00:00`)) })}
			</p>
		{/if}
		{#if item.changed}
			<p>{m.vault_changed_at({ time: timeFormat.format(new Date(item.changed)) })}</p>
		{/if}
		{#if item.source === 'shared'}
			<p class="flex items-center gap-2">
				<Lock size={14} class="text-ok" aria-hidden="true" />
				{m.catalog_access({ role: ROLE_LABELS[item.credential.role]() })}
			</p>
		{:else}
			<p class="flex items-center gap-2">
				<Lock size={14} class="text-ok" aria-hidden="true" />
				{m.vault_personal_only()}
			</p>
		{/if}
	</div>
</section>

{#snippet fileItem(file: { id: string; name: string; size: number })}
	<span class="inline-flex items-center gap-1">
		<button
			type="button"
			class="text-accent hover:underline"
			title={m.viewer_show_file({ name: file.name })}
			onclick={() => view(file)}
		>
			{file.name} ({size(file.size)})
		</button>
		<button
			type="button"
			class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
			title={m.viewer_download_file({ name: file.name })}
			aria-label={m.viewer_download_file({ name: file.name })}
			onclick={() => download(file)}
		>
			<Download size={13} aria-hidden="true" />
		</button>
	</span>
{/snippet}

<Dialog bind:open={viewerOpen} title={viewing?.name ?? ''} large>
	{#if viewerOpen && viewing}
		<FileViewer name={viewing.name} load={viewing.load} ondownload={viewing.download} />
	{/if}
</Dialog>
