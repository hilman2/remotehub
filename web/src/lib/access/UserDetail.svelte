<script lang="ts">
	/**
	 * One person on the Access page (#178): sign-in state and actions, groups,
	 * roles, and what they reach as a folder tree with the grant behind each
	 * row. Grants to the person are changed or removed here; auditors only
	 * look.
	 */
	import Ban from '@lucide/svelte/icons/ban';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Clock from '@lucide/svelte/icons/clock';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Monitor from '@lucide/svelte/icons/monitor';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import ShieldOff from '@lucide/svelte/icons/shield-off';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import X from '@lucide/svelte/icons/x';
	import {
		loadPrincipalGrants,
		type AccessGroup,
		type AccessUser,
		type PrincipalGrant
	} from '$lib/api/access';
	import {
		ROLES,
		addGrant,
		allows,
		loadTree,
		removeGrant,
		type Principal,
		type Role,
		type Tree
	} from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { addMember, removeMember } from '$lib/api/groups';
	import { userReport, type Reach, type UserReport } from '$lib/api/reports';
	import { assignRole, revokeRole } from '$lib/api/roles';
	import { resetFactor } from '$lib/api/secondFactor';
	import {
		blockUser,
		deleteUser,
		endSessions,
		issueRecovery,
		unblockUser,
		type OneTimeCode
	} from '$lib/api/users';
	import { ROLE_LABELS } from '$lib/catalog/labels';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale, getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { session, type Role as SiteRole } from '$lib/session.svelte';
	import { goto } from '$app/navigation';
	import { SvelteSet } from 'svelte/reactivity';
	import CodeView from './CodeView.svelte';
	import GrantDialog from './GrantDialog.svelte';
	import { SITE_ROLE_LABELS, USER_KIND_LABELS } from './labels';
	import { accessHref } from './links';
	import MailChoice from './MailChoice.svelte';

	let {
		user,
		groups,
		admin,
		mailServer,
		onchanged
	}: {
		user: AccessUser;
		/** Every group, for adding the person to an own one. */
		groups: AccessGroup[];
		admin: boolean;
		mailServer: boolean;
		onchanged: () => void;
	} = $props();

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	let report = $state<UserReport | null>(null);
	let direct = $state<PrincipalGrant[]>([]);
	let tree = $state<Tree | null>(null);
	let error = $state<string | null>(null);

	async function load() {
		const [r, g] = await Promise.all([
			userReport(user.id),
			user.principal ? loadPrincipalGrants(user.principal) : Promise.resolve(null)
		]);
		if (r.ok) report = r.data;
		else error = errorMessage(r.code);
		if (g?.ok) direct = g.data;
	}

	$effect(() => {
		void user.id;
		report = null;
		direct = [];
		load();
	});

	$effect(() => {
		if (admin && !tree) loadTree().then((t) => t.ok && (tree = t.data));
	});

	/** The person as grants and memberships name them. */
	const principal = $derived<Principal | null>(
		user.principal
			? { kind: 'user', sid: user.principal, name: user.display_name, detail: null }
			: null
	);
	const isSelf = $derived(
		user.username === session.user?.username && user.kind === session.user?.kind
	);

	async function changed() {
		await load();
		onchanged();
	}

	// ── Actions on the account ─────────────────────────────────────────────

	type Change = 'block' | 'unblock' | 'sessions' | 'recovery' | 'factor' | 'delete';
	let confirming = $state<Change | null>(null);
	let code = $state<OneTimeCode | null>(null);
	let dialogOpen = $state(false);
	let dialogError = $state<string | null>(null);
	let sendMail = $state(true);
	let language = $state<string>(getLocale());
	let busy = $state(false);

	const questions: Record<Change, (name: string) => string> = {
		block: (name) => m.users_block_confirm({ name }),
		unblock: (name) => m.users_unblock_confirm({ name }),
		sessions: (name) => m.users_end_sessions_confirm({ name }),
		recovery: (name) => m.users_recovery_confirm({ name }),
		factor: (name) => m.users_reset_factor_confirm({ name }),
		delete: (name) => m.users_delete_confirm({ name })
	};
	const actions: Record<Change, () => string> = {
		block: m.users_block,
		unblock: m.users_unblock,
		sessions: m.users_end_sessions,
		recovery: m.users_recovery,
		factor: m.users_reset_factor,
		delete: m.catalog_delete
	};

	function confirm(change: Change) {
		confirming = change;
		code = null;
		dialogError = null;
		dialogOpen = true;
	}

	async function apply(change: Change) {
		busy = true;
		const result = await {
			block: blockUser,
			unblock: unblockUser,
			sessions: endSessions,
			recovery: (id: string) => issueRecovery(id, { send_mail: mailServer && sendMail, language }),
			factor: resetFactor,
			delete: deleteUser
		}[change](user.id);
		busy = false;
		if (!result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		if (change === 'recovery') code = result.data as OneTimeCode;
		else dialogOpen = false;
		if (change === 'delete') {
			onchanged();
			await goto(accessHref({ tab: 'users' }));
			return;
		}
		onchanged();
	}

	// ── Groups and roles ────────────────────────────────────────────────────

	const ownGroups = $derived(
		groups.filter((g) => g.source === 'own' && !user.groups.some((ug) => ug.sid === g.sid))
	);
	let joining = $state('');

	async function join() {
		const group = groups.find((g) => g.sid === joining);
		if (!group?.id || !principal) return;
		const result = await addMember(group.id, principal);
		if (!result.ok) error = errorMessage(result.code);
		joining = '';
		onchanged();
	}

	async function leave(sid: string) {
		const group = groups.find((g) => g.sid === sid);
		if (!group?.id || !user.principal) return;
		const result = await removeMember(group.id, user.principal);
		if (!result.ok) error = errorMessage(result.code);
		onchanged();
	}

	let giving = $state<SiteRole | ''>('');
	const SITE_ROLES: SiteRole[] = ['administrator', 'auditor', 'security_officer'];

	async function give() {
		if (!giving || !principal) return;
		const result = await assignRole(giving, principal);
		if (!result.ok) error = errorMessage(result.code);
		giving = '';
		onchanged();
	}

	async function takeRole(role: SiteRole) {
		if (!user.principal) return;
		const result = await revokeRole(role, user.principal);
		if (!result.ok) error = errorMessage(result.code);
		onchanged();
	}

	// ── What the person reaches ─────────────────────────────────────────────

	interface Row {
		key: string;
		reach: Reach;
		depth: number;
		/** Folders only: whether something is below it. */
		parent: boolean;
	}

	let query = $state('');
	let atLeast = $state<Role>('list');
	/** The folders whose content is hidden, by key. */
	const closed = new SvelteSet<string>();

	const keyOf = (path: string[], name: string) => [...path, name].join('\u0000');
	const shownKinds = (r: Reach) => r.object.kind !== 'credential';

	const rows = $derived.by<Row[]>(() => {
		if (!report) return [];
		const all = report.reach.filter(shownKinds);
		const q = query.trim().toLowerCase();
		const matches = (r: Reach) =>
			allows(r.role, atLeast) &&
			(q === '' ||
				r.name.toLowerCase().includes(q) ||
				r.path.some((p) => p.toLowerCase().includes(q)));
		// A match keeps the folders above it, so it stays in its place.
		const keep = new Set(
			all
				.filter(matches)
				.flatMap((r) => [
					keyOf(r.path, r.name),
					...r.path.map((name, i) => keyOf(r.path.slice(0, i), name))
				])
		);
		const parents = new Set(all.map((r) => r.path.join('\u0000')));
		// Folders before devices, then by name, level by level.
		const sorted = [...all].sort((a, b) => {
			const pa = [...a.path, a.object.kind === 'folder' ? `0${a.name}` : `1${a.name}`];
			const pb = [...b.path, b.object.kind === 'folder' ? `0${b.name}` : `1${b.name}`];
			return pa.join('\u0000').localeCompare(pb.join('\u0000'), getLocale());
		});
		return sorted
			.filter((r) => keep.has(keyOf(r.path, r.name)))
			.filter((r) => !r.path.some((_, i) => closed.has(keyOf(r.path.slice(0, i), r.path[i]))))
			.map((r) => ({
				key: keyOf(r.path, r.name),
				reach: r,
				depth: r.path.length,
				parent: r.object.kind === 'folder' && parents.has([...r.path, r.name].join('\u0000'))
			}));
	});

	const counts = $derived({
		folders: report?.reach.filter((r) => r.object.kind === 'folder').length ?? 0,
		devices: report?.reach.filter((r) => r.object.kind === 'device').length ?? 0
	});

	function toggle(key: string) {
		if (closed.has(key)) closed.delete(key);
		else closed.add(key);
	}

	function collapseAll() {
		const parents = rows.filter((r) => r.parent).map((r) => r.key);
		closed.clear();
		for (const key of parents) closed.add(key);
	}

	/** The grant to the person themselves on this object, without end. */
	const ownGrant = (r: Reach) =>
		direct.find(
			(g) => g.object.kind === r.object.kind && g.object.id === r.object.id && !g.expires_at
		);

	async function setRole(r: Reach, role: Role) {
		if (!principal) return;
		const result = await addGrant(r.object.kind, r.object.id, principal, role);
		if (!result.ok) error = errorMessage(result.code);
		await changed();
	}

	async function ungrant(id: string) {
		const result = await removeGrant(id);
		if (!result.ok) error = errorMessage(result.code);
		await changed();
	}

	function through(r: Reach): string {
		return r.reasons
			.map((reason) =>
				reason.inherited
					? m.permissions_reason_inherited({
							role: ROLE_LABELS[reason.role](),
							principal: reason.principal_name,
							on: reason.on_name
						})
					: m.permissions_reason({
							role: ROLE_LABELS[reason.role](),
							principal: reason.principal_name
						})
			)
			.join('; ');
	}

	// Granting: to the person or one of their groups.
	let granting = $state(false);
	const targets = $derived([
		...(principal ? [{ principal, note: m.access_this_person() }] : []),
		...user.groups.map((g) => ({
			principal: { kind: 'group' as const, sid: g.sid, name: g.name, detail: null },
			note: g.source === 'own' ? m.access_group_own() : m.access_group_directory()
		}))
	]);
	const currentAccess = (_: Principal, kind: string, id: string) => {
		const r = report?.reach.find((x) => x.object.kind === kind && x.object.id === id);
		return r ? m.access_has({ role: ROLE_LABELS[r.role]() }) : null;
	};

	const chip = 'inline-flex items-center gap-1.5 rounded-md border border-line px-2 py-0.5 text-xs';
	const quiet = 'rounded-lg px-2.5 py-1.5 text-sm hover:bg-surface-2';
</script>

<nav class="text-sm text-ink-2" aria-label={m.access_breadcrumb()}>
	<a class="hover:underline" href={accessHref({ tab: 'users' })}>{m.access_tab_users()}</a>
	<span aria-hidden="true">/</span>
	<span>{user.display_name}</span>
</nav>

<section
	class="mt-4 flex flex-wrap items-start gap-4 rounded-card border border-line bg-surface p-5"
>
	<div class="min-w-0 flex-1">
		<h2 class="text-2xl font-semibold">{user.display_name}</h2>
		<p class="text-sm text-ink-2">
			{user.username}{#if user.email && user.email !== user.username}
				· {user.email}{/if}
		</p>
		<div class="mt-3 flex flex-wrap gap-2">
			<span class={chip}>{USER_KIND_LABELS[user.kind]()}</span>
			<span class={chip}>
				{#if user.blocked}
					<Ban size={13} class="text-critical" aria-hidden="true" />{m.users_blocked()}
				{:else}
					<CircleCheck size={13} class="text-ok" aria-hidden="true" />{m.users_active()}
				{/if}
			</span>
			<span class={chip}>
				{#if user.second_factor}
					<ShieldCheck size={13} class="text-ok" aria-hidden="true" />{m.access_factor_on()}
				{:else}
					<ShieldOff size={13} class="text-ink-3" aria-hidden="true" />{m.access_factor_off()}
				{/if}
			</span>
			<span class={chip}>
				{m.access_last_sign_in({
					time: user.last_sign_in_at
						? time.format(new Date(user.last_sign_in_at))
						: m.connectors_never()
				})}
			</span>
			<span class={chip}>{m.access_sessions({ count: user.sessions })}</span>
		</div>
	</div>
	{#if admin}
		<div class="flex flex-wrap gap-2">
			{#if user.sessions > 0 && !isSelf}
				<button type="button" class={quiet} onclick={() => confirm('sessions')}>
					<LogOut size={14} class="mr-1 inline" aria-hidden="true" />{m.users_end_sessions()}
				</button>
			{/if}
			{#if user.kind === 'local'}
				<button type="button" class={quiet} onclick={() => confirm('recovery')}>
					<KeyRound size={14} class="mr-1 inline" aria-hidden="true" />{m.users_recovery()}
				</button>
			{/if}
			{#if user.kind === 'directory' && user.second_factor}
				<button type="button" class={quiet} onclick={() => confirm('factor')}>
					<ShieldOff size={14} class="mr-1 inline" aria-hidden="true" />{m.users_reset_factor()}
				</button>
			{/if}
			{#if user.blocked}
				<button type="button" class={quiet} onclick={() => confirm('unblock')}>
					{m.users_unblock()}
				</button>
			{:else if !isSelf}
				<button type="button" class="{quiet} text-critical" onclick={() => confirm('block')}>
					<Ban size={14} class="mr-1 inline" aria-hidden="true" />{m.users_block()}
				</button>
			{/if}
			{#if user.kind === 'local' && !isSelf}
				<button type="button" class="{quiet} text-critical" onclick={() => confirm('delete')}>
					<Trash2 size={14} class="mr-1 inline" aria-hidden="true" />{m.catalog_delete()}
				</button>
			{/if}
		</div>
	{/if}
</section>

{#if error}
	<p class="mt-4 text-sm text-critical" role="alert">{error}</p>
{/if}

<div class="mt-6 grid gap-6 lg:grid-cols-[20rem_1fr]">
	<div class="flex flex-col gap-6">
		<section class="rounded-card border border-line bg-surface p-4" aria-labelledby="user-groups">
			<h3 id="user-groups" class="font-semibold">{m.groups_title()}</h3>
			{#if user.groups.length === 0}
				<p class="mt-2 text-sm text-ink-2">{m.access_no_groups()}</p>
			{:else}
				<ul class="mt-2 divide-y divide-line">
					{#each user.groups as g (g.sid)}
						<li class="flex items-center gap-2 py-1.5 text-sm">
							<div class="min-w-0 flex-1">
								<a
									class="text-accent hover:underline"
									href={accessHref({ tab: 'groups', group: g.sid })}>{g.name}</a
								>
								<div class="text-xs text-ink-3">
									{g.source === 'directory'
										? m.access_group_directory()
										: g.via
											? m.access_group_own_via({ name: g.via.name })
											: m.access_group_own()}
								</div>
							</div>
							{#if admin && g.source === 'own' && !g.via}
								<button
									type="button"
									class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
									aria-label={m.access_leave_group({ name: user.display_name, group: g.name })}
									onclick={() => leave(g.sid)}
								>
									<X size={14} aria-hidden="true" />
								</button>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
			{#if user.kind === 'directory'}
				<p class="mt-2 text-xs text-ink-3">{m.access_directory_groups_note()}</p>
			{/if}
			{#if admin && principal && ownGroups.length > 0}
				<div class="mt-3 flex gap-2">
					<select
						class="min-w-0 flex-1 rounded-lg border border-line bg-page px-2 py-1.5 text-sm"
						aria-label={m.access_add_to_group()}
						bind:value={joining}
					>
						<option value="">{m.access_add_to_group()}</option>
						{#each ownGroups as g (g.sid)}
							<option value={g.sid}>{g.name}</option>
						{/each}
					</select>
					<button type="button" class={quiet} disabled={!joining} onclick={join}>
						{m.groups_add_member()}
					</button>
				</div>
			{/if}
		</section>

		<section class="rounded-card border border-line bg-surface p-4" aria-labelledby="user-roles">
			<h3 id="user-roles" class="font-semibold">{m.roles_title()}</h3>
			{#if user.roles.length === 0}
				<p class="mt-2 text-sm text-ink-2">{m.access_no_role()}</p>
			{:else}
				<ul class="mt-2 divide-y divide-line">
					{#each user.roles as r (`${r.role}${r.via.sid}`)}
						<li class="flex items-center gap-2 py-1.5 text-sm">
							<div class="min-w-0 flex-1">
								{SITE_ROLE_LABELS[r.role]()}
								<div class="text-xs text-ink-3">
									{r.via.sid === user.principal
										? m.access_role_own()
										: m.access_role_via({ name: r.via.name })}
								</div>
							</div>
							{#if admin && r.via.sid === user.principal}
								<button
									type="button"
									class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
									aria-label={m.access_take_role({ role: SITE_ROLE_LABELS[r.role]() })}
									onclick={() => takeRole(r.role)}
								>
									<X size={14} aria-hidden="true" />
								</button>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
			{#if admin && principal}
				<div class="mt-3 flex gap-2">
					<select
						class="min-w-0 flex-1 rounded-lg border border-line bg-page px-2 py-1.5 text-sm"
						aria-label={m.access_give_role()}
						bind:value={giving}
					>
						<option value="">{m.access_give_role()}</option>
						{#each SITE_ROLES as role (role)}
							<option value={role}>{SITE_ROLE_LABELS[role]()}</option>
						{/each}
					</select>
					<button type="button" class={quiet} disabled={!giving} onclick={give}>
						{m.groups_add_member()}
					</button>
				</div>
			{/if}
		</section>
	</div>

	<section class="rounded-card border border-line bg-surface p-4" aria-labelledby="user-reach">
		<div class="flex flex-wrap items-start justify-between gap-3">
			<div>
				<h3 id="user-reach" class="font-semibold">{m.access_col_access()}</h3>
				<p class="text-sm text-ink-2">
					{report?.administrator
						? m.permissions_administrator()
						: m.access_reaches({ folders: counts.folders, devices: counts.devices })}
				</p>
			</div>
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
		{#if report?.groups_from === 'last_sign_in'}
			<p class="mt-2 text-xs text-ink-3">{m.permissions_groups_last_sign_in()}</p>
		{:else if report?.groups_from === 'none' && user.kind === 'directory'}
			<p class="mt-2 text-xs text-ink-3">{m.permissions_groups_unknown()}</p>
		{/if}
		<div class="mt-3 flex flex-wrap items-center gap-3">
			<input
				type="search"
				class="min-w-48 flex-1 rounded-lg border border-line bg-page px-2 py-1.5 text-sm"
				placeholder={m.access_filter_objects()}
				aria-label={m.access_filter_objects()}
				bind:value={query}
			/>
			<label class="flex items-center gap-2 text-sm text-ink-2">
				{m.access_at_least()}
				<select class="rounded-lg border border-line bg-page px-2 py-1.5" bind:value={atLeast}>
					{#each ROLES as r (r)}
						<option value={r}>{ROLE_LABELS[r]()}</option>
					{/each}
				</select>
			</label>
			<button type="button" class={quiet} onclick={() => closed.clear()}>
				{m.access_expand_all()}
			</button>
			<button type="button" class={quiet} onclick={collapseAll}>{m.access_collapse_all()}</button>
		</div>
		<div class="mt-3 overflow-x-auto">
			<table class="w-full text-left text-sm" data-testid="person-report">
				<thead class="border-b border-line text-ink-2">
					<tr>
						<th class="px-2 py-2 font-medium">{m.access_col_object()}</th>
						<th class="px-2 py-2 font-medium">{m.access_col_access()}</th>
						<th class="px-2 py-2 font-medium">{m.permissions_through()}</th>
					</tr>
				</thead>
				<tbody>
					{#each rows as row (row.key)}
						{@const r = row.reach}
						{@const own = ownGrant(r)}
						<tr class="border-b border-line last:border-0">
							<td class="px-2 py-1.5">
								<span class="flex items-center gap-1.5" style="padding-left: {row.depth * 1.25}rem">
									{#if row.parent}
										<button
											type="button"
											class="rounded p-0.5 hover:bg-surface-2"
											aria-expanded={!closed.has(row.key)}
											aria-label={closed.has(row.key)
												? m.access_expand({ name: r.name })
												: m.access_collapse({ name: r.name })}
											onclick={() => toggle(row.key)}
										>
											{#if closed.has(row.key)}
												<ChevronRight size={14} aria-hidden="true" />
											{:else}
												<ChevronDown size={14} aria-hidden="true" />
											{/if}
										</button>
									{:else}
										<span class="w-5"></span>
									{/if}
									{#if r.object.kind === 'folder'}
										<FolderIcon size={15} class="text-ink-3" aria-hidden="true" />
									{:else}
										<Monitor size={15} class="text-ink-3" aria-hidden="true" />
									{/if}
									<span class="font-medium">{r.name}</span>
								</span>
							</td>
							<td class="px-2 py-1.5">
								{#if admin && own}
									<span class="flex items-center gap-1">
										<select
											class="rounded-md border border-line bg-page px-1.5 py-0.5 text-sm"
											aria-label={m.access_role_on({ name: r.name })}
											value={own.role}
											onchange={(e) => setRole(r, e.currentTarget.value as Role)}
										>
											{#each ROLES as role (role)}
												<option value={role}>{ROLE_LABELS[role]()}</option>
											{/each}
										</select>
										<button
											type="button"
											class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-critical"
											aria-label={m.access_ungrant({ name: r.name })}
											onclick={() => ungrant(own.id)}
										>
											<X size={14} aria-hidden="true" />
										</button>
									</span>
								{:else}
									<span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-xs">
										{ROLE_LABELS[r.role]()}
									</span>
								{/if}
							</td>
							<td class="px-2 py-1.5 text-xs text-ink-2">
								{through(r)}
								{#each direct.filter((g) => g.expires_at && g.object.id === r.object.id) as g (g.id)}
									<span class="ml-1 inline-flex items-center gap-1">
										<Clock size={12} aria-hidden="true" />
										{m.grants_until({ time: time.format(new Date(g.expires_at ?? '')) })}
									</span>
								{/each}
							</td>
						</tr>
					{:else}
						<tr>
							<td colspan="3" class="px-2 py-6 text-center text-ink-2">
								{report ? m.permissions_nothing() : ''}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>
</div>

{#if admin && tree}
	<GrantDialog bind:open={granting} {targets} {tree} current={currentAccess} ongranted={changed} />
{/if}

<Dialog
	bind:open={dialogOpen}
	title={code
		? m.users_code_title({ name: user.display_name })
		: confirming
			? actions[confirming]()
			: ''}
>
	{#if code}
		<CodeView {code} onclose={() => (dialogOpen = false)} />
	{:else if confirming}
		{@const change = confirming}
		<p class="text-sm">{questions[change](user.display_name)}</p>
		{#if change === 'recovery' && mailServer}
			<MailChoice bind:sendMail bind:language />
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
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm font-medium {change === 'block' || change === 'delete'
					? 'bg-critical text-white'
					: 'bg-accent text-accent-ink'}"
				disabled={busy}
				onclick={() => apply(change)}
			>
				{actions[change]()}
			</button>
		</div>
	{/if}
</Dialog>
