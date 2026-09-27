<script lang="ts">
	/**
	 * Creates a collection of shared credentials, or renames and moves one
	 * (#190). Both take `manage` on the collection it goes into; the top
	 * level is for administrators.
	 */
	import { untrack } from 'svelte';
	import {
		allows,
		createCollection,
		updateCollection,
		type Collection,
		type Tree
	} from '$lib/api/catalog';
	import { problemMessage } from '$lib/api/errors';
	import { collectionPath, collectionsBelow } from '$lib/catalog/tree';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let {
		tree,
		collection = null,
		parent = null,
		onsaved,
		oncancel
	}: {
		tree: Tree;
		/** The one to change; null: a new one. */
		collection?: Collection | null;
		/** Where a new one goes; null: the top level. */
		parent?: string | null;
		onsaved: (id: string) => void;
		oncancel: () => void;
	} = $props();

	const start = untrack(() => $state.snapshot(collection));
	let name = $state(start?.name ?? '');
	// '' stands for the top level: a select holds strings.
	let into = $state(untrack(() => (start ? start.parent_id : parent) ?? ''));
	let error = $state<string | null>(null);
	let busy = $state(false);

	/** Where it may go: collections one manages, never into itself. */
	const places = $derived.by(() => {
		const own = start ? collectionsBelow(tree, start.id) : new Set<string>();
		return tree.collections
			.filter((c) => allows(c.role, 'manage') && !own.has(c.id))
			.map((c) => ({
				id: c.id,
				path: collectionPath(tree, c.id)
					.map((p) => p.name)
					.join(' / ')
			}))
			.sort((a, b) => a.path.localeCompare(b.path, getLocale(), { sensitivity: 'base' }));
	});
	/** The top level is offered to who may create there, and kept where it is. */
	const topLevel = $derived(
		tree.may_create_top_level || (start !== null && start.parent_id === null)
	);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		const target = into || null;
		const result = start
			? await updateCollection(start.id, {
					name,
					// Sent only when it moves: staying needs no right on the parent.
					...(target !== start.parent_id ? { parent_id: target } : {})
				})
			: await createCollection(target, name);
		busy = false;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		const made = result.data as { id?: string } | undefined;
		onsaved(made?.id ?? start?.id ?? '');
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
</script>

<form onsubmit={submit}>
	<label class="block text-sm font-medium" for="collection-name">{m.field_name()}</label>
	<input id="collection-name" class={field} required maxlength="200" bind:value={name} />
	<label class="mt-3 block text-sm font-medium" for="collection-parent">
		{m.vault_collection_parent()}
	</label>
	<select id="collection-parent" class={field} bind:value={into}>
		{#if topLevel}
			<option value="">{m.vault_top_level()}</option>
		{/if}
		{#each places as place (place.id)}
			<option value={place.id}>{place.path}</option>
		{/each}
	</select>
	{#if error}
		<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
	{/if}
	<div class="mt-5 flex justify-end gap-2">
		<button
			type="button"
			class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
			onclick={oncancel}
		>
			{m.action_cancel()}
		</button>
		<button
			type="submit"
			class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
			disabled={busy}
		>
			{start ? m.action_save() : m.action_create()}
		</button>
	</div>
</form>
