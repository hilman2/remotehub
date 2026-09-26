<script lang="ts">
	/**
	 * Directory groups and remotehub's own groups (#178): a searchable list,
	 * and for one group its members, the groups it is in, its roles and its
	 * grants. Own groups are created, renamed and filled here.
	 */
	import Clock from '@lucide/svelte/icons/clock';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { loadPrincipalGrants, type AccessGroup, type PrincipalGrant } from '$lib/api/access';
	import { loadTree, removeGrant, type Principal, type Tree } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { addMember, createGroup, deleteGroup, removeMember, updateGroup } from '$lib/api/groups';
	import { KIND_LABELS, ROLE_LABELS } from '$lib/catalog/labels';
	import PrincipalName from '$lib/catalog/PrincipalName.svelte';
	import PrincipalPicker from '$lib/catalog/PrincipalPicker.svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { goto } from '$app/navigation';
	import GrantDialog from './GrantDialog.svelte';
	import { SITE_ROLE_LABELS } from './labels';
	import { accessHref } from './links';

	let {
		groups,
		selected,
		admin,
		onchanged
	}: {
		groups: AccessGroup[];
		/** The SID of the group whose detail is shown. */
		selected: string | undefined;
		admin: boolean;
		onchanged: () => void;
	} = $props();

	const PAGE = 25;
	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	let query = $state('');
	let source = $state<'any' | 'directory' | 'own'>('any');
	const listed = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return groups.filter(
			(g) =>
				(source === 'any' || g.source === source) && (q === '' || g.name.toLowerCase().includes(q))
		);
	});

	const group = $derived(groups.find((g) => g.sid === selected));
	const sourceLabel = (g: AccessGroup) =>
		g.source === 'own' ? m.access_group_own() : m.access_group_directory();

	// ── The detail ──────────────────────────────────────────────────────────

	let memberQuery = $state('');
	let memberPage = $state(0);
	let grants = $state<PrincipalGrant[]>([]);
	let tree = $state<Tree | null>(null);
	let error = $state<string | null>(null);

	async function loadGrants(sid: string) {
		const result = await loadPrincipalGrants(sid);
		if (result.ok) grants = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		const sid = group?.sid;
		memberQuery = '';
		memberPage = 0;
		grants = [];
		error = null;
		if (sid) loadGrants(sid);
	});

	$effect(() => {
		if (admin && group && !tree) loadTree().then((t) => t.ok && (tree = t.data));
	});

	const members = $derived.by(() => {
		const q = memberQuery.trim().toLowerCase();
		return (group?.members ?? []).filter((mb) => q === '' || mb.name.toLowerCase().includes(q));
	});
	const memberPages = $derived(Math.max(1, Math.ceil(members.length / PAGE)));
	$effect(() => {
		void memberQuery;
		memberPage = 0;
	});

	async function ungrant(id: string) {
		const result = await removeGrant(id);
		if (!result.ok) error = errorMessage(result.code);
		if (group) await loadGrants(group.sid);
		onchanged();
	}

	async function dropMember(sid: string) {
		if (!group?.id) return;
		const result = await removeMember(group.id, sid);
		if (!result.ok) error = errorMessage(result.code);
		onchanged();
	}

	// ── Dialogs: new, edit, delete, add member, grant ───────────────────────

	type Open = { type: 'new' } | { type: 'edit' } | { type: 'delete' } | { type: 'member' };
	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let dialogError = $state<string | null>(null);
	let name = $state('');
	let description = $state('');
	let chosen = $state<Principal | null>(null);
	let granting = $state(false);

	function show(next: Open) {
		dialogError = null;
		name = next.type === 'edit' ? (group?.name ?? '') : '';
		description = next.type === 'edit' ? (group?.description ?? '') : '';
		chosen = null;
		open = next;
		dialogOpen = true;
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		let result;
		if (open?.type === 'new') {
			result = await createGroup(name, description);
			if (result.ok) {
				dialogOpen = false;
				onchanged();
				await goto(accessHref({ tab: 'groups', group: `group:${result.data.id}` }));
				return;
			}
		} else if (open?.type === 'edit' && group?.id) {
			result = await updateGroup(group.id, name, description);
		} else if (open?.type === 'member' && group?.id && chosen) {
			result = await addMember(group.id, chosen);
		} else if (open?.type === 'delete' && group?.id) {
			result = await deleteGroup(group.id);
			if (result.ok) {
				dialogOpen = false;
				onchanged();
				await goto(accessHref({ tab: 'groups' }));
				return;
			}
		}
		if (result && !result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		dialogOpen = false;
		onchanged();
	}

	const titles = $derived.by(() => {
		switch (open?.type) {
			case 'new':
				return m.groups_new();
			case 'edit':
				return m.groups_edit();
			case 'delete':
				return m.catalog_delete();
			case 'member':
				return m.groups_members_title({ name: group?.name ?? '' });
			default:
				return '';
		}
	});

	const field = 'rounded-lg border border-line bg-page px-2 py-1.5 text-sm';
	const quiet = 'rounded-lg px-2.5 py-1.5 text-sm hover:bg-surface-2';
	const card = 'rounded-card border border-line bg-surface p-4';
</script>

{#if group}
	<nav class="text-sm text-ink-2" aria-label={m.access_breadcrumb()}>
		<a class="hover:underline" href={accessHref({ tab: 'groups' })}>{m.access_tab_groups()}</a>
		<span aria-hidden="true">/</span>
		<span>{group.name}</span>
	</nav>
	<section class="mt-4 flex flex-wrap items-start gap-4 {card}">
		<div class="min-w-0 flex-1">
			<h2 class="text-2xl font-semibold">{group.name}</h2>
			<p class="text-sm text-ink-2">{sourceLabel(group)}</p>
			{#if group.description}
				<p class="mt-2 text-sm">{group.description}</p>
			{/if}
		</div>
		{#if admin && group.source === 'own'}
			<div class="flex gap-2">
				<button type="button" class={quiet} onclick={() => show({ type: 'edit' })}>
					{m.groups_edit()}
				</button>
				<button
					type="button"
					class="{quiet} text-critical"
					onclick={() => show({ type: 'delete' })}
				>
					{m.catalog_delete()}
				</button>
			</div>
		{/if}
	</section>
	{#if error}
		<p class="mt-4 text-sm text-critical" role="alert">{error}</p>
	{/if}

	<div class="mt-6 grid gap-6 lg:grid-cols-[1fr_20rem]">
		<section class={card} aria-labelledby="group-members">
			<div class="flex flex-wrap items-center gap-3">
				<h3 id="group-members" class="font-semibold">
					{m.groups_members()}
					<span class="text-sm font-normal text-ink-3 tabular-nums">{group.members.length}</span>
				</h3>
				<input
					type="search"
					class="{field} ml-auto min-w-48"
					placeholder={m.access_search_members()}
					aria-label={m.access_search_members()}
					bind:value={memberQuery}
				/>
				{#if admin && group.source === 'own'}
					<button type="button" class={quiet} onclick={() => show({ type: 'member' })}>
						<Plus size={14} class="mr-1 inline" aria-hidden="true" />{m.groups_add_member()}
					</button>
				{/if}
			</div>
			{#if group.source === 'directory'}
				<p class="mt-2 text-xs text-ink-3">{m.access_directory_members_note()}</p>
			{/if}
			{#if members.length === 0}
				<p class="mt-3 text-sm text-ink-2">{m.groups_members_none()}</p>
			{:else}
				<ul class="mt-2 divide-y divide-line">
					{#each members.slice(memberPage * PAGE, memberPage * PAGE + PAGE) as member (member.sid)}
						<li class="flex items-center gap-2 py-1.5 text-sm">
							{#if member.user_id}
								<a
									class="text-accent hover:underline"
									href={accessHref({ tab: 'users', user: member.user_id })}>{member.name}</a
								>
							{:else if member.kind === 'group'}
								<a
									class="text-accent hover:underline"
									href={accessHref({ tab: 'groups', group: member.sid })}
									><PrincipalName kind="group" name={member.name} /></a
								>
							{:else}
								<PrincipalName kind={member.kind} name={member.name} />
							{/if}
							{#if admin && group.source === 'own'}
								<button
									type="button"
									class="ml-auto rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
									aria-label={m.groups_remove_member({ name: member.name })}
									onclick={() => dropMember(member.sid)}
								>
									<X size={14} aria-hidden="true" />
								</button>
							{/if}
						</li>
					{/each}
				</ul>
				{#if memberPages > 1}
					<nav
						class="mt-2 flex items-center justify-end gap-2 text-sm"
						aria-label={m.access_pages()}
					>
						<button
							type="button"
							class="{quiet} disabled:opacity-40"
							disabled={memberPage === 0}
							onclick={() => (memberPage -= 1)}>{m.access_previous()}</button
						>
						<span class="text-ink-2 tabular-nums"
							>{m.access_page({ page: memberPage + 1, pages: memberPages })}</span
						>
						<button
							type="button"
							class="{quiet} disabled:opacity-40"
							disabled={memberPage >= memberPages - 1}
							onclick={() => (memberPage += 1)}>{m.access_next()}</button
						>
					</nav>
				{/if}
			{/if}
		</section>

		<div class="flex flex-col gap-6">
			<section class={card} aria-labelledby="group-member-of">
				<h3 id="group-member-of" class="font-semibold">{m.access_member_of()}</h3>
				{#if group.member_of.length === 0}
					<p class="mt-2 text-sm text-ink-2">{m.access_in_no_group()}</p>
				{:else}
					<ul class="mt-2 space-y-1 text-sm">
						{#each group.member_of as g (g.sid)}
							<li>
								<a
									class="text-accent hover:underline"
									href={accessHref({ tab: 'groups', group: g.sid })}>{g.name}</a
								>
							</li>
						{/each}
					</ul>
				{/if}
			</section>
			<section class={card} aria-labelledby="group-roles">
				<h3 id="group-roles" class="font-semibold">{m.roles_title()}</h3>
				{#if group.roles.length === 0}
					<p class="mt-2 text-sm text-ink-2">{m.access_no_role()}</p>
				{:else}
					<ul class="mt-2 space-y-1 text-sm">
						{#each group.roles as role (role)}
							<li>{SITE_ROLE_LABELS[role]()}</li>
						{/each}
					</ul>
				{/if}
			</section>
		</div>
	</div>

	<section class="mt-6 {card}" aria-labelledby="group-grants">
		<div class="flex items-center justify-between gap-3">
			<h3 id="group-grants" class="font-semibold">{m.access_grants()}</h3>
			{#if admin && tree}
				<button
					type="button"
					class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
					onclick={() => (granting = true)}
				>
					{m.access_grant()}
				</button>
			{/if}
		</div>
		{#if grants.length === 0}
			<p class="mt-2 text-sm text-ink-2">{m.access_no_grants()}</p>
		{:else}
			<ul class="mt-2 divide-y divide-line">
				{#each grants as g (g.id)}
					<li class="flex flex-wrap items-center gap-3 py-1.5 text-sm">
						<span class="text-ink-3">{KIND_LABELS[g.object.kind]()}</span>
						<span class="font-medium">{g.name}</span>
						{#if g.path.length > 0}
							<span class="text-xs text-ink-3">{g.path.join(' / ')}</span>
						{/if}
						<span class="ml-auto">{ROLE_LABELS[g.role]()}</span>
						{#if g.expires_at}
							<span class="inline-flex items-center gap-1 text-xs text-ink-3">
								<Clock size={12} aria-hidden="true" />
								{m.grants_until({ time: time.format(new Date(g.expires_at)) })}
							</span>
						{/if}
						{#if admin}
							<button
								type="button"
								class="rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
								onclick={() => ungrant(g.id)}
							>
								{m.grants_remove()}
							</button>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	{#if admin && tree}
		<GrantDialog
			bind:open={granting}
			targets={[
				{
					principal: { kind: 'group', sid: group.sid, name: group.name, detail: null },
					note: sourceLabel(group)
				}
			]}
			{tree}
			ongranted={() => {
				if (group) loadGrants(group.sid);
				onchanged();
			}}
		/>
	{/if}
{:else}
	<div class="flex flex-wrap items-end gap-3">
		<input
			type="search"
			class="{field} min-w-64 flex-1"
			placeholder={m.access_search_groups()}
			aria-label={m.access_search_groups()}
			bind:value={query}
		/>
		<label class="flex flex-col text-xs text-ink-2">
			{m.users_col_source()}
			<select class={field} bind:value={source}>
				<option value="any">{m.access_any()}</option>
				<option value="directory">{m.access_group_directory()}</option>
				<option value="own">{m.access_group_own()}</option>
			</select>
		</label>
		<span class="ml-auto text-sm text-ink-2 tabular-nums">
			{m.access_count({ shown: listed.length, total: groups.length })}
		</span>
		{#if admin}
			<button
				type="button"
				class="inline-flex items-center gap-2 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
				onclick={() => show({ type: 'new' })}
			>
				<Plus size={16} aria-hidden="true" />
				{m.groups_new()}
			</button>
		{/if}
	</div>
	<p class="mt-2 text-sm text-ink-2">{m.groups_hint()}</p>
	<div class="mt-4 overflow-x-auto rounded-card border border-line bg-surface">
		<table class="w-full text-left text-sm">
			<thead class="border-b border-line text-ink-2">
				<tr>
					<th class="px-4 py-2 font-medium">{m.field_name()}</th>
					<th class="px-4 py-2 font-medium">{m.users_col_source()}</th>
					<th class="px-4 py-2 font-medium">{m.groups_members()}</th>
					<th class="px-4 py-2 font-medium">{m.roles_title()}</th>
					<th class="px-4 py-2 font-medium">{m.access_grants()}</th>
				</tr>
			</thead>
			<tbody>
				{#each listed as g (g.sid)}
					<tr class="border-b border-line last:border-0" data-testid="group-row">
						<td class="px-4 py-2">
							<a
								class="font-medium text-accent hover:underline"
								href={accessHref({ tab: 'groups', group: g.sid })}>{g.name}</a
							>
							{#if g.description}
								<div class="text-xs text-ink-3">{g.description}</div>
							{/if}
						</td>
						<td class="px-4 py-2 text-ink-2">{sourceLabel(g)}</td>
						<td class="px-4 py-2 text-ink-2 tabular-nums">{g.members.length}</td>
						<td class="px-4 py-2">
							{#each g.roles as role (role)}
								<span class="mr-1 rounded-md bg-surface-2 px-1.5 py-0.5 text-xs"
									>{SITE_ROLE_LABELS[role]()}</span
								>
							{/each}
						</td>
						<td class="px-4 py-2 text-ink-2 tabular-nums">{g.grants}</td>
					</tr>
				{:else}
					<tr><td colspan="5" class="px-4 py-6 text-center text-ink-2">{m.groups_empty()}</td></tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

<Dialog bind:open={dialogOpen} title={titles}>
	<form onsubmit={submit}>
		{#if open?.type === 'new' || open?.type === 'edit'}
			<label class="block text-sm font-medium" for="group-name">{m.field_name()}</label>
			<input
				id="group-name"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				required
				maxlength="200"
				bind:value={name}
			/>
			<label class="mt-4 block text-sm font-medium" for="group-description"
				>{m.field_description()}</label
			>
			<input
				id="group-description"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				maxlength="500"
				bind:value={description}
			/>
		{:else if open?.type === 'member'}
			{#key dialogOpen}
				<PrincipalPicker
					id="group-member"
					bind:chosen
					onerror={(message) => (dialogError = message)}
				/>
			{/key}
		{:else if open?.type === 'delete'}
			<p class="text-sm">{m.groups_delete_confirm({ name: group?.name ?? '' })}</p>
		{/if}
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
				class="rounded-lg px-3 py-1.5 text-sm font-medium disabled:opacity-50 {open?.type ===
				'delete'
					? 'bg-critical text-white'
					: 'bg-accent text-accent-ink'}"
				disabled={open?.type === 'member' && !chosen}
			>
				{open?.type === 'new'
					? m.action_create()
					: open?.type === 'delete'
						? m.catalog_delete()
						: open?.type === 'member'
							? m.groups_add_member()
							: m.action_save()}
			</button>
		</div>
	</form>
</Dialog>
