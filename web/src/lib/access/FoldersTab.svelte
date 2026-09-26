<script lang="ts">
	/**
	 * Folders (#178): a searchable folder tree, and for one folder who reaches
	 * it, through which grant, with the members of remotehub's own groups.
	 * Administrators change the folder's grants here.
	 */
	import FolderIcon from '@lucide/svelte/icons/folder';
	import { errorMessage } from '$lib/api/errors';
	import {
		folderReport,
		loadFolders,
		type FolderChoice,
		type FolderReport
	} from '$lib/api/reports';
	import Grants from '$lib/catalog/Grants.svelte';
	import { ROLE_LABELS } from '$lib/catalog/labels';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { accessHref } from './links';

	let { selected, admin }: { selected: string | undefined; admin: boolean } = $props();

	let folders = $state<FolderChoice[]>([]);
	let report = $state<FolderReport | null>(null);
	let query = $state('');
	let error = $state<string | null>(null);

	$effect(() => {
		loadFolders().then((result) => {
			if (result.ok) folders = result.data;
			else error = errorMessage(result.code);
		});
	});

	async function loadReport(id: string) {
		const result = await folderReport(id);
		if (result.ok) report = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		report = null;
		if (selected) loadReport(selected);
	});

	const joined = (path: string[]) => path.join('\u0000');
	const shown = $derived.by(() => {
		const q = query.trim().toLowerCase();
		// A match keeps the folders above it, so it stays in its place.
		const keep = new Set(
			folders
				.filter((f) => q === '' || f.path[f.path.length - 1].toLowerCase().includes(q))
				.flatMap((f) => f.path.map((_, i) => joined(f.path.slice(0, i + 1))))
		);
		return [...folders]
			.filter((f) => keep.has(joined(f.path)))
			.sort((a, b) => joined(a.path).localeCompare(joined(b.path), getLocale()));
	});

	const cell = 'px-3 py-2 align-top';
</script>

{#if error}
	<p class="mb-4 text-sm text-critical" role="alert">{error}</p>
{/if}

<div class="grid gap-6 lg:grid-cols-[20rem_1fr]">
	<section
		class="rounded-card border border-line bg-surface p-4"
		aria-label={m.access_tab_folders()}
	>
		<input
			type="search"
			class="w-full rounded-lg border border-line bg-page px-2 py-1.5 text-sm"
			placeholder={m.access_search_folders()}
			aria-label={m.access_search_folders()}
			bind:value={query}
		/>
		<ul class="mt-3 max-h-[36rem] overflow-y-auto">
			{#each shown as folder (folder.id)}
				<li>
					<a
						class="flex items-center gap-1.5 rounded-md px-2 py-1 text-sm hover:bg-surface-2 {folder.id ===
						selected
							? 'bg-surface-2 font-medium'
							: ''}"
						style="padding-left: {(folder.path.length - 1) * 1.1 + 0.5}rem"
						href={accessHref({ tab: 'folders', folder: folder.id })}
						aria-current={folder.id === selected ? 'page' : undefined}
					>
						<FolderIcon size={14} class="shrink-0 text-ink-3" aria-hidden="true" />
						{folder.path[folder.path.length - 1]}
					</a>
				</li>
			{:else}
				<li class="px-2 py-4 text-sm text-ink-2">{m.access_no_objects()}</li>
			{/each}
		</ul>
	</section>

	<section class="rounded-card border border-line bg-surface p-4">
		{#if !selected}
			<p class="text-sm text-ink-2">{m.access_pick_folder()}</p>
		{:else if report}
			<h2 class="text-xl font-semibold">{report.folder.path.join(' / ')}</h2>
			<p class="mt-1 text-sm text-ink-2">{m.permissions_admins_reach_all()}</p>
			{#if report.holders.length === 0}
				<p class="mt-4 text-sm text-ink-3">{m.permissions_nobody()}</p>
			{:else}
				<table class="mt-4 w-full text-sm" data-testid="folder-report">
					<thead class="text-left text-ink-3">
						<tr>
							<th class={cell}>{m.permissions_who()}</th>
							<th class={cell}>{m.access_col_access()}</th>
							<th class={cell}>{m.permissions_granted_on()}</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-line">
						{#each report.holders as holder, index (index)}
							<tr>
								<td class={cell}>
									<span class="font-medium">{holder.principal_name}</span>
									{#if holder.members.length > 0}
										<span class="block text-xs text-ink-3">
											{m.permissions_members({ names: holder.members.join(', ') })}
										</span>
									{/if}
								</td>
								<td class={cell}>{ROLE_LABELS[holder.role]()}</td>
								<td class={cell}>
									{holder.on_path.join(' / ')}
									{#if holder.inherited}
										<span class="block text-xs text-ink-3">{m.permissions_inherited()}</span>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
			{#if admin}
				<div class="mt-6 border-t border-line pt-4">
					{#key selected}
						<Grants kind="folder" id={selected} />
					{/key}
				</div>
			{/if}
		{/if}
	</section>
</div>
