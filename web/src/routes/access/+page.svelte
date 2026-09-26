<script lang="ts">
	/**
	 * Access (#178): users, groups, roles and folders in one place, in place
	 * of the pages Users and Permissions. Administrators change things here;
	 * auditors see the same without the controls.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import { page } from '$app/state';
	import { loadAccess, type Overview } from '$lib/api/access';
	import { errorMessage } from '$lib/api/errors';
	import FoldersTab from '$lib/access/FoldersTab.svelte';
	import GroupsTab from '$lib/access/GroupsTab.svelte';
	import { accessHref, accessView, type AccessTab } from '$lib/access/links';
	import RolesTab from '$lib/access/RolesTab.svelte';
	import UserDetail from '$lib/access/UserDetail.svelte';
	import UsersTab from '$lib/access/UsersTab.svelte';
	import { m } from '$lib/paraglide/messages';
	import { isAuditor, loadMethods, session } from '$lib/session.svelte';

	let overview = $state<Overview | null>(null);
	let error = $state<string | null>(null);
	let local = $state(false);
	let mailServer = $state(false);

	const allowed = $derived(isAuditor(session.user));
	const admin = $derived(session.user?.admin ?? false);
	const view = $derived(accessView(page.url.searchParams));

	async function load() {
		const result = await loadAccess();
		if (result.ok) overview = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		if (!allowed) return;
		load();
		if (admin) {
			loadMethods().then((result) => {
				local = result.ok && result.data.local;
				mailServer = result.ok && result.data.mail;
			});
		}
	});

	const tabs = $derived<{ tab: AccessTab; label: string; count: number | null }[]>([
		{ tab: 'users', label: m.access_tab_users(), count: overview?.users.length ?? null },
		{ tab: 'groups', label: m.access_tab_groups(), count: overview?.groups.length ?? null },
		{ tab: 'roles', label: m.access_tab_roles(), count: null },
		{ tab: 'folders', label: m.access_tab_folders(), count: null }
	]);
	const user = $derived(view.user ? overview?.users.find((u) => u.id === view.user) : undefined);
</script>

<h1 class="text-4xl font-semibold">{m.nav_access()}</h1>
<p class="mt-2 text-sm text-ink-2">{m.access_intro()}</p>

{#if !allowed}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else}
	<nav class="mt-6 flex gap-1 border-b border-line" aria-label={m.access_sections()}>
		{#each tabs as t (t.tab)}
			<a
				href={accessHref({ tab: t.tab })}
				class="-mb-px inline-flex items-center gap-2 border-b-2 px-3 py-2 text-sm font-medium {view.tab ===
				t.tab
					? 'border-accent text-ink'
					: 'border-transparent text-ink-2 hover:text-ink'}"
				aria-current={view.tab === t.tab ? 'page' : undefined}
			>
				{t.label}
				{#if t.count !== null}
					<span class="rounded-full bg-surface-2 px-1.5 text-xs text-ink-2 tabular-nums"
						>{t.count}</span
					>
				{/if}
			</a>
		{/each}
	</nav>
	{#if !admin}
		<p class="mt-3 text-xs text-ink-3">{m.access_read_only()}</p>
	{/if}

	<div class="mt-6">
		{#if error}
			<p class="flex items-center gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="text-critical" aria-hidden="true" />
				{error}
			</p>
		{:else if view.tab === 'folders'}
			<FoldersTab selected={view.folder} {admin} />
		{:else if !overview}
			<p class="text-sm text-ink-2">{m.access_loading()}</p>
		{:else if view.tab === 'users'}
			{#if user}
				<UserDetail {user} groups={overview.groups} {admin} {mailServer} onchanged={load} />
			{:else}
				<UsersTab
					users={overview.users}
					groups={overview.groups}
					{admin}
					{local}
					{mailServer}
					onchanged={load}
				/>
			{/if}
		{:else if view.tab === 'groups'}
			<GroupsTab groups={overview.groups} selected={view.group} {admin} onchanged={load} />
		{:else}
			<RolesTab users={overview.users} {admin} onchanged={load} />
		{/if}
	</div>
{/if}
