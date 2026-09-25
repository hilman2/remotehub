<script lang="ts">
	/**
	 * The shared credentials this user sees (#98), with the folders they lie
	 * in. Their secrets are shown or copied with `reveal`, as on the devices
	 * page; they do not need the personal vault to be open.
	 */
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { allows, loadTree, type Credential, type Tree } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import RevealSecret from '$lib/catalog/RevealSecret.svelte';
	import { pathTo } from '$lib/catalog/tree';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { queryKey } from '$lib/search/rank';
	import EntryDetails from './EntryDetails.svelte';
	import { icon } from './icons';

	let { query = '' }: { query?: string } = $props();

	let tree = $state<Tree | null>(null);
	let error = $state<string | null>(null);
	let expanded = $state<string | null>(null);

	$effect(() => {
		loadTree().then((result) => {
			if (result.ok) tree = result.data;
			else error = errorMessage(result.code);
		});
	});

	const path = (credential: Credential) =>
		tree
			? pathTo(tree, credential.folder_id)
					.map((folder) => folder.name)
					.join(' / ')
			: '';

	/** By folder, then by name; narrowed by the page's search. */
	const listed = $derived.by(() => {
		if (!tree) return [];
		const wanted = queryKey(query);
		const collator = new Intl.Collator(getLocale());
		return tree.credentials
			.map((credential) => ({ credential, path: path(credential) }))
			.filter(
				({ credential, path }) =>
					!wanted ||
					queryKey(`${credential.name} ${credential.username} ${credential.url} ${path}`).includes(
						wanted
					)
			)
			.sort(
				(a, b) =>
					collator.compare(a.path, b.path) || collator.compare(a.credential.name, b.credential.name)
			);
	});
</script>

<section class="mt-10" aria-labelledby="shared-title">
	<h2 id="shared-title" class="text-lg font-semibold">{m.vault_shared()}</h2>
	<p class="mt-1 text-sm text-ink-2">{m.vault_shared_hint()}</p>
	{#if error}
		<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
	{:else if tree && listed.length === 0}
		<p class="mt-3 text-sm text-ink-3">{m.vault_shared_none()}</p>
	{:else}
		<ul class="mt-3 divide-y divide-line rounded-card border border-line bg-surface empty:hidden">
			{#each listed as { credential, path } (credential.id)}
				{@const Icon = icon(credential.icon)}
				<li class="px-4 py-3 text-sm" data-testid="shared-entry">
					<button
						type="button"
						class="flex w-full items-center gap-3 text-left"
						aria-expanded={expanded === credential.id}
						onclick={() => (expanded = expanded === credential.id ? null : credential.id)}
					>
						<Icon size={16} class="shrink-0 text-warning" aria-hidden="true" />
						<span class="min-w-0">
							<span class="block font-medium">{credential.name}</span>
							<span class="block truncate text-ink-2">{path} · {credential.username}</span>
						</span>
						<ChevronRight
							size={16}
							class="ml-auto shrink-0 text-ink-3 transition-transform {expanded === credential.id
								? 'rotate-90'
								: ''}"
							aria-hidden="true"
						/>
					</button>
					{#if expanded === credential.id}
						<div class="mt-3 flex flex-col gap-3 pl-7">
							<EntryDetails
								url={credential.url}
								notes={credential.notes}
								fields={credential.fields}
							/>
							{#if allows(credential.role, 'reveal')}
								<RevealSecret owner="credentials" id={credential.id} />
							{/if}
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</section>
