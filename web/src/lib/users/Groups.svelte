<script lang="ts">
	/**
	 * Groups of remotehub's own (#105): created, renamed and deleted here,
	 * with their members. Access is granted to them like to directory groups.
	 */
	import Pencil from '@lucide/svelte/icons/pencil';
	import Plus from '@lucide/svelte/icons/plus';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import UsersIcon from '@lucide/svelte/icons/users';
	import X from '@lucide/svelte/icons/x';
	import type { Principal } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import {
		addMember,
		createGroup,
		deleteGroup,
		loadGroups,
		removeMember,
		updateGroup,
		type Group
	} from '$lib/api/groups';
	import PrincipalName from '$lib/catalog/PrincipalName.svelte';
	import PrincipalPicker from '$lib/catalog/PrincipalPicker.svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import SettingsMenu, { type MenuItem } from '$lib/components/SettingsMenu.svelte';
	import { m } from '$lib/paraglide/messages';

	type Open =
		| { type: 'edit'; group: Group | null }
		| { type: 'members'; id: string }
		| { type: 'delete'; group: Group };

	let groups = $state<Group[]>([]);
	let error = $state<string | null>(null);
	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let dialogError = $state<string | null>(null);
	let name = $state('');
	let description = $state('');
	let chosen = $state<Principal | null>(null);
	/** Remounts the picker after each member added. */
	let picks = $state(0);

	async function load() {
		const result = await loadGroups();
		if (result.ok) groups = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		load();
	});

	function show(next: Open) {
		dialogError = null;
		name = next.type === 'edit' ? (next.group?.name ?? '') : '';
		description = next.type === 'edit' ? (next.group?.description ?? '') : '';
		chosen = null;
		open = next;
		dialogOpen = true;
	}

	/** The group the members dialog shows, as last loaded. */
	const shown = $derived(
		open?.type === 'members' ? groups.find((g) => g.id === (open as { id: string }).id) : undefined
	);

	function menu(group: Group): MenuItem[] {
		return [
			{
				label: m.groups_members(),
				icon: UsersIcon,
				onselect: () => show({ type: 'members', id: group.id })
			},
			{ label: m.groups_edit(), icon: Pencil, onselect: () => show({ type: 'edit', group }) },
			{
				label: m.catalog_delete(),
				icon: Trash2,
				danger: true,
				onselect: () => show({ type: 'delete', group })
			}
		];
	}

	async function done(result: { ok: boolean; code?: string }, close = true) {
		if (!result.ok) {
			dialogError = errorMessage(result.code ?? 'internal');
			return false;
		}
		dialogError = null;
		if (close) dialogOpen = false;
		await load();
		return true;
	}

	async function save(event: SubmitEvent, group: Group | null) {
		event.preventDefault();
		await done(
			group ? await updateGroup(group.id, name, description) : await createGroup(name, description)
		);
	}

	async function add(id: string) {
		if (!chosen) return;
		if (await done(await addMember(id, chosen), false)) {
			chosen = null;
			picks += 1;
		}
	}

	const title = $derived.by(() => {
		switch (open?.type) {
			case 'edit':
				return open.group ? m.groups_edit() : m.groups_new();
			case 'members':
				return m.groups_members_title({ name: shown?.name ?? '' });
			case 'delete':
				return m.catalog_delete();
			default:
				return '';
		}
	});
</script>

<section class="mt-12" aria-labelledby="groups-title">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<h2 id="groups-title" class="text-2xl font-semibold">{m.groups_title()}</h2>
		<button
			type="button"
			class="inline-flex items-center gap-2 rounded-lg border border-line-strong px-3 py-1.5 text-sm font-medium hover:bg-surface-2"
			onclick={() => show({ type: 'edit', group: null })}
		>
			<Plus size={16} aria-hidden="true" />
			{m.groups_new()}
		</button>
	</div>
	<p class="mt-2 text-sm text-ink-2">{m.groups_hint()}</p>

	{#if error}
		<p class="mt-4 text-sm text-critical" role="alert">{error}</p>
	{:else if groups.length === 0}
		<p class="mt-4 text-sm text-ink-2">{m.groups_empty()}</p>
	{:else}
		<div class="mt-4 overflow-x-auto rounded-card border border-line bg-surface">
			<table class="w-full text-left text-sm">
				<thead class="border-b border-line text-ink-2">
					<tr>
						<th class="px-4 py-2 font-medium">{m.field_name()}</th>
						<th class="px-4 py-2 font-medium">{m.groups_members()}</th>
						<th class="px-4 py-2"><span class="sr-only">{m.users_actions()}</span></th>
					</tr>
				</thead>
				<tbody>
					{#each groups as group (group.id)}
						<tr class="border-b border-line last:border-0" data-testid="group-row">
							<td class="px-4 py-2">
								<div class="font-medium">{group.name}</div>
								{#if group.description}
									<div class="text-xs text-ink-3">{group.description}</div>
								{/if}
							</td>
							<td class="px-4 py-2 text-ink-2">
								{#if group.members.length === 0}
									{m.groups_members_none()}
								{:else}
									{group.members.map((member) => member.name).join(', ')}
								{/if}
							</td>
							<td class="px-4 py-2">
								<div class="flex justify-end">
									<SettingsMenu
										label={m.users_actions_for({ name: group.name })}
										items={menu(group)}
									/>
								</div>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>

<Dialog bind:open={dialogOpen} {title}>
	{#if open?.type === 'edit'}
		{@const group = open.group}
		<form onsubmit={(event) => save(event, group)}>
			<label class="block text-sm font-medium" for="group-name">{m.field_name()}</label>
			<input
				id="group-name"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				required
				maxlength="200"
				autocomplete="off"
				bind:value={name}
			/>
			<label class="mt-4 block text-sm font-medium" for="group-description">
				{m.field_description()}
			</label>
			<input
				id="group-description"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				maxlength="500"
				autocomplete="off"
				bind:value={description}
			/>
			{#if dialogError}
				<p class="mt-3 text-sm text-critical" role="alert">{dialogError}</p>
			{/if}
			<div class="mt-5 flex justify-end gap-2">
				<button
					type="button"
					class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
					onclick={() => (dialogOpen = false)}
				>
					{m.action_cancel()}
				</button>
				<button
					type="submit"
					class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
				>
					{group ? m.action_save() : m.action_create()}
				</button>
			</div>
		</form>
	{:else if open?.type === 'members' && shown}
		{@const group = shown}
		{#if group.members.length === 0}
			<p class="text-sm text-ink-2">{m.groups_members_none()}</p>
		{:else}
			<ul class="divide-y divide-line rounded-lg border border-line">
				{#each group.members as member (member.sid)}
					<li class="flex items-center gap-2 px-3 py-1.5 text-sm">
						<PrincipalName kind={member.kind} name={member.name} />
						<button
							type="button"
							class="ml-auto rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
							title={m.groups_remove_member({ name: member.name })}
							onclick={async () => done(await removeMember(group.id, member.sid), false)}
						>
							<X size={14} aria-hidden="true" />
							<span class="sr-only">{m.groups_remove_member({ name: member.name })}</span>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
		<div class="mt-4">
			{#key picks}
				<PrincipalPicker
					id="group-member"
					bind:chosen
					onerror={(message) => (dialogError = message)}
					accepts={(principal) => !principal.sid.startsWith('group:')}
				/>
			{/key}
		</div>
		{#if dialogError}
			<p class="mt-3 text-sm text-critical" role="alert">{dialogError}</p>
		{/if}
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={() => (dialogOpen = false)}
			>
				{m.action_close()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
				disabled={!chosen}
				onclick={() => add(group.id)}
			>
				{m.groups_add_member()}
			</button>
		</div>
	{:else if open?.type === 'delete'}
		{@const group = open.group}
		<p class="text-sm">{m.groups_delete_confirm({ name: group.name })}</p>
		{#if dialogError}
			<p class="mt-3 text-sm text-critical" role="alert">{dialogError}</p>
		{/if}
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={() => (dialogOpen = false)}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={async () => done(await deleteGroup(group.id))}
			>
				{m.catalog_delete()}
			</button>
		</div>
	{/if}
</Dialog>
