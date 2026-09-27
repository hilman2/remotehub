<script lang="ts">
	/**
	 * A shared credential in the vault (#190): what it is, its secret for
	 * someone with `reveal`, its files and versions.
	 */
	import Clock from '@lucide/svelte/icons/clock';
	import Lock from '@lucide/svelte/icons/lock';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { allows, type Credential, type Tree } from '$lib/api/catalog';
	import { requestableRoles } from '$lib/api/requests';
	import CredentialExtras from '$lib/catalog/CredentialExtras.svelte';
	import { ROLE_LABELS } from '$lib/catalog/labels';
	import RevealSecret from '$lib/catalog/RevealSecret.svelte';
	import { collectionPath } from '$lib/catalog/tree';
	import { m } from '$lib/paraglide/messages';
	import EntryDetails from './EntryDetails.svelte';
	import { icon } from './icons';

	let {
		tree,
		credential,
		onedit,
		ondelete,
		onrequest,
		onchange
	}: {
		tree: Tree;
		credential: Credential;
		onedit: () => void;
		ondelete: () => void;
		onrequest: () => void;
		/** After a file was added or removed. */
		onchange: () => void;
	} = $props();

	const Icon = $derived(icon(credential.icon));
	const path = $derived(
		collectionPath(tree, credential.collection_id)
			.map((c) => c.name)
			.join(' / ')
	);

	const button =
		'inline-flex h-9 items-center gap-2 rounded-lg border border-line-strong bg-surface px-3 text-sm hover:bg-surface-2';
	const card = 'flex flex-col gap-3 rounded-card border border-line bg-surface p-5';
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
			<h2 class="text-2xl font-semibold break-words">{credential.name}</h2>
			<p class="text-sm text-ink-3">{m.vault_in_collection({ path })}</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			{#if requestableRoles(credential.role).length > 0}
				<button type="button" class={button} onclick={onrequest}>
					<Clock size={16} aria-hidden="true" />
					{m.request_access()}
				</button>
			{/if}
			{#if allows(credential.role, 'edit')}
				<button type="button" class={button} onclick={onedit}>
					<Pencil size={16} aria-hidden="true" />
					{m.catalog_edit()}
				</button>
				<button type="button" class="{button} text-critical" onclick={ondelete}>
					<Trash2 size={16} aria-hidden="true" />
					<span class="sr-only">{m.catalog_delete()}</span>
				</button>
			{/if}
		</div>
	</div>

	<div class="flex flex-wrap gap-2">
		{#if credential.username}
			<span class="chip font-mono">
				{credential.domain ? `${credential.domain}\\${credential.username}` : credential.username}
			</span>
		{/if}
		<span class="chip">{m.catalog_access({ role: ROLE_LABELS[credential.role]() })}</span>
		<span class="chip">{m.credential_version({ version: credential.version })}</span>
	</div>

	<section class={card}>
		<h3 class="eyebrow">{m.field_password()}</h3>
		<p class="flex items-center gap-2 font-medium">
			<Lock size={16} class="text-ok" aria-hidden="true" />
			{m.credential_hidden()}
		</p>
		{#if allows(credential.role, 'reveal')}
			{#key credential.id}
				<RevealSecret owner="credentials" id={credential.id} />
			{/key}
		{/if}
	</section>

	{#if credential.url || credential.notes || credential.fields.length > 0}
		<section class={card}>
			<EntryDetails url={credential.url} notes={credential.notes} fields={credential.fields} />
		</section>
	{/if}

	<section class={card}>
		{#key credential.id}
			<CredentialExtras {credential} {onchange} />
		{/key}
	</section>
</div>
