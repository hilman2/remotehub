<script lang="ts">
	/**
	 * Grants access on several folders or devices at once (#178), to a person
	 * or to one of their groups, until further notice or until a day.
	 * `current` names the access someone already has on an object, so the
	 * choice is made knowing it.
	 */
	import FolderIcon from '@lucide/svelte/icons/folder';
	import Monitor from '@lucide/svelte/icons/monitor';
	import X from '@lucide/svelte/icons/x';
	import {
		ROLES,
		addGrant,
		type ObjectKind,
		type Principal,
		type Role,
		type Tree
	} from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { ROLE_LABELS } from '$lib/catalog/labels';
	import { pathTo } from '$lib/catalog/tree';
	import Dialog from '$lib/components/Dialog.svelte';
	import { m } from '$lib/paraglide/messages';
	import { ROLE_HINTS } from './labels';

	let {
		open = $bindable(false),
		targets,
		tree,
		current = () => null,
		ongranted
	}: {
		open: boolean;
		/** Whom to grant to: the person first, then their groups. */
		targets: { principal: Principal; note: string }[];
		tree: Tree;
		/** The access the chosen principal has on an object already, in words. */
		current?: (principal: Principal, kind: ObjectKind, id: string) => string | null;
		ongranted: () => void;
	} = $props();

	interface Item {
		key: string;
		kind: 'folder' | 'device';
		id: string;
		name: string;
		path: string;
	}

	let target = $state(0);
	let query = $state('');
	let picked = $state<Item[]>([]);
	let role = $state<Role>('connect');
	let ends = $state(false);
	let until = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);

	// Opening starts afresh.
	$effect(() => {
		if (!open) return;
		target = 0;
		query = '';
		picked = [];
		role = 'connect';
		ends = false;
		until = '';
		error = null;
	});

	const pathOf = (folderId: string | null) =>
		pathTo(tree, folderId)
			.map((f) => f.name)
			.join(' / ') || m.access_top_level();

	const items = $derived<Item[]>([
		...tree.folders.map((f) => ({
			key: `folder:${f.id}`,
			kind: 'folder' as const,
			id: f.id,
			name: f.name,
			path: pathOf(f.parent_id)
		})),
		...tree.devices.map((d) => ({
			key: `device:${d.id}`,
			kind: 'device' as const,
			id: d.id,
			name: d.name,
			path: pathOf(d.folder_id)
		}))
	]);
	const results = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return items
			.filter(
				(i) => q === '' || i.name.toLowerCase().includes(q) || i.path.toLowerCase().includes(q)
			)
			.slice(0, 100);
	});
	const chosen = $derived(targets[target]?.principal ?? null);

	function toggle(item: Item) {
		picked = picked.some((p) => p.key === item.key)
			? picked.filter((p) => p.key !== item.key)
			: [...picked, item];
	}

	async function grant(event: SubmitEvent) {
		event.preventDefault();
		if (!chosen || picked.length === 0) return;
		// The end of the chosen day, in the browser's time zone.
		const expiresAt = ends && until ? new Date(`${until}T23:59:59`).toISOString() : null;
		busy = true;
		for (const item of picked) {
			const result = await addGrant(item.kind, item.id, chosen, role, expiresAt);
			if (!result.ok) {
				busy = false;
				error = errorMessage(result.code);
				ongranted();
				return;
			}
		}
		busy = false;
		open = false;
		ongranted();
	}

	const radio = (active: boolean) =>
		`flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 ${active ? 'border-accent bg-accent/10' : 'border-line'}`;
</script>

<Dialog bind:open title={m.access_grant()} wide>
	<form onsubmit={grant} class="flex flex-col gap-5">
		<p class="text-sm text-ink-2">{m.access_grant_hint()}</p>

		<fieldset>
			<legend class="text-sm font-medium">{m.access_grant_to()}</legend>
			<div class="mt-2 grid gap-2 sm:grid-cols-3">
				{#each targets as t, index (t.principal.sid)}
					<label class={radio(index === target)}>
						<input type="radio" name="grant-to" value={index} bind:group={target} />
						<span class="min-w-0">
							<span class="block truncate font-medium">{t.principal.name}</span>
							<span class="block text-xs text-ink-3">{t.note}</span>
						</span>
					</label>
				{/each}
			</div>
			<p class="mt-2 text-xs text-ink-3">{m.access_grant_group_hint()}</p>
		</fieldset>

		<div>
			<label class="block text-sm font-medium" for="grant-objects">{m.access_objects()}</label>
			<input
				id="grant-objects"
				type="search"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				placeholder={m.access_objects_search()}
				bind:value={query}
			/>
			{#if picked.length > 0}
				<div class="mt-2 flex flex-wrap items-center gap-1.5">
					<span class="text-sm text-ink-2">{m.access_selected({ count: picked.length })}</span>
					{#each picked as item (item.key)}
						<span
							class="inline-flex items-center gap-1 rounded-full bg-accent/10 py-0.5 pr-1 pl-2.5 text-sm"
						>
							{item.name}
							<button
								type="button"
								class="rounded-full p-0.5 hover:bg-surface-2"
								aria-label={m.access_unselect({ name: item.name })}
								onclick={() => toggle(item)}
							>
								<X size={13} aria-hidden="true" />
							</button>
						</span>
					{/each}
				</div>
			{/if}
			<div
				class="mt-2 h-56 overflow-y-auto rounded-lg border border-line"
				role="group"
				aria-label={m.access_objects()}
			>
				{#each results as item (item.key)}
					{@const has = chosen ? current(chosen, item.kind, item.id) : null}
					<label
						class="flex cursor-pointer items-center gap-3 border-b border-line px-3 py-2 last:border-0 hover:bg-surface-2"
					>
						<input
							type="checkbox"
							checked={picked.some((p) => p.key === item.key)}
							onchange={() => toggle(item)}
						/>
						{#if item.kind === 'folder'}
							<FolderIcon size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
						{:else}
							<Monitor size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
						{/if}
						<span class="min-w-0 flex-1">
							<span class="block truncate font-medium">{item.name}</span>
							<span class="block truncate text-xs text-ink-3">{item.path}</span>
						</span>
						{#if has}
							<span class="shrink-0 text-xs text-ink-2">{has}</span>
						{/if}
					</label>
				{:else}
					<p class="px-3 py-6 text-center text-sm text-ink-2">{m.access_no_objects()}</p>
				{/each}
			</div>
		</div>

		<fieldset>
			<legend class="text-sm font-medium">{m.access_col_access()}</legend>
			<div class="mt-2 flex flex-col gap-1">
				{#each ROLES as r (r)}
					<label class={radio(r === role)}>
						<input type="radio" name="grant-role" value={r} bind:group={role} />
						<span class="w-32 shrink-0 font-medium">{ROLE_LABELS[r]()}</span>
						<span class="text-sm text-ink-2">{ROLE_HINTS[r]()}</span>
					</label>
				{/each}
			</div>
		</fieldset>

		<div class="flex flex-wrap items-center gap-3">
			<label class="flex items-center gap-2 text-sm font-medium">
				<input type="checkbox" bind:checked={ends} />
				{m.access_ends()}
			</label>
			<input
				type="date"
				class="rounded-lg border border-line bg-page px-2 py-1.5"
				aria-label={m.access_ends()}
				disabled={!ends}
				required={ends}
				bind:value={until}
			/>
			<span class="text-sm text-ink-3">{ends ? m.access_ends_hint() : m.access_no_end_hint()}</span>
		</div>

		{#if error}
			<p class="text-sm text-critical" role="alert">{error}</p>
		{/if}
		<div class="flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={() => (open = false)}
			>
				{m.action_cancel()}
			</button>
			<button
				type="submit"
				class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
				disabled={busy || picked.length === 0 || !chosen}
			>
				{picked.length === 0
					? m.access_pick_objects()
					: m.access_grant_submit({ role: ROLE_LABELS[role](), count: picked.length })}
			</button>
		</div>
	</form>
</Dialog>
