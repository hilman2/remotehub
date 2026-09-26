<script lang="ts">
	/**
	 * Everyone who signs in, with their groups and role (#178): searched by
	 * name, username or group, filtered and paged in the browser. A name
	 * opens the person's detail.
	 */
	import Ban from '@lucide/svelte/icons/ban';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import ShieldOff from '@lucide/svelte/icons/shield-off';
	import UserPlus from '@lucide/svelte/icons/user-plus';
	import type { AccessGroup, AccessUser } from '$lib/api/access';
	import { errorMessage } from '$lib/api/errors';
	import { inviteUser, type OneTimeCode, type UserKind } from '$lib/api/users';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale, getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { SITE_ROLE_LABELS, USER_KIND_LABELS } from './labels';
	import CodeView from './CodeView.svelte';
	import MailChoice from './MailChoice.svelte';
	import { accessHref } from './links';

	let {
		users,
		groups,
		admin,
		local,
		mailServer,
		onchanged
	}: {
		users: AccessUser[];
		groups: AccessGroup[];
		/** May change things; an auditor only looks. */
		admin: boolean;
		/** Local accounts are set up: people can be invited. */
		local: boolean;
		mailServer: boolean;
		onchanged: () => void;
	} = $props();

	const PAGE = 25;

	let query = $state('');
	let source = $state<'any' | UserKind>('any');
	let status = $state<'any' | 'active' | 'blocked'>('any');
	let factor = $state<'any' | 'on' | 'off'>('any');
	let group = $state('any');
	let page = $state(0);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	const filtered = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return users.filter(
			(u) =>
				(q === '' ||
					u.display_name.toLowerCase().includes(q) ||
					u.username.toLowerCase().includes(q) ||
					u.groups.some((g) => g.name.toLowerCase().includes(q))) &&
				(source === 'any' || u.kind === source) &&
				(status === 'any' || u.blocked === (status === 'blocked')) &&
				(factor === 'any' || u.second_factor === (factor === 'on')) &&
				(group === 'any' || u.groups.some((g) => g.sid === group))
		);
	});
	const pages = $derived(Math.max(1, Math.ceil(filtered.length / PAGE)));
	const shown = $derived(filtered.slice(page * PAGE, page * PAGE + PAGE));
	const narrowed = $derived(
		query !== '' || source !== 'any' || status !== 'any' || factor !== 'any' || group !== 'any'
	);

	// A new filter starts at the first page.
	$effect(() => {
		void [query, source, status, factor, group];
		page = 0;
	});

	function clear() {
		query = '';
		source = 'any';
		status = 'any';
		factor = 'any';
		group = 'any';
	}

	// Inviting a local account.
	let dialogOpen = $state(false);
	let code = $state<{ code: OneTimeCode; name: string } | null>(null);
	let email = $state('');
	let name = $state('');
	let sendMail = $state(true);
	let language = $state<string>(getLocale());
	let error = $state<string | null>(null);
	let busy = $state(false);

	function showInvite() {
		code = null;
		email = '';
		name = '';
		error = null;
		dialogOpen = true;
	}

	async function invite(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		const result = await inviteUser(email, name, {
			send_mail: mailServer && sendMail,
			language
		});
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		code = { code: result.data, name: name.trim() || email.trim() };
		onchanged();
	}

	const field = 'rounded-lg border border-line bg-page px-2 py-1.5 text-sm';
	const cell = 'px-4 py-2 align-top';
</script>

<div class="flex flex-wrap items-end gap-3">
	<input
		type="search"
		class="{field} min-w-64 flex-1"
		placeholder={m.access_search_users()}
		aria-label={m.access_search_users()}
		bind:value={query}
	/>
	<label class="flex flex-col text-xs text-ink-2">
		{m.users_col_source()}
		<select class={field} bind:value={source}>
			<option value="any">{m.access_any()}</option>
			{#each ['directory', 'local', 'break_glass'] as const as kind (kind)}
				<option value={kind}>{USER_KIND_LABELS[kind]()}</option>
			{/each}
		</select>
	</label>
	<label class="flex flex-col text-xs text-ink-2">
		{m.users_col_state()}
		<select class={field} bind:value={status}>
			<option value="any">{m.access_any()}</option>
			<option value="active">{m.users_active()}</option>
			<option value="blocked">{m.users_blocked()}</option>
		</select>
	</label>
	<label class="flex flex-col text-xs text-ink-2">
		{m.users_col_second_factor()}
		<select class={field} bind:value={factor}>
			<option value="any">{m.access_any()}</option>
			<option value="on">{m.users_factor_on()}</option>
			<option value="off">{m.users_factor_off()}</option>
		</select>
	</label>
	<label class="flex flex-col text-xs text-ink-2">
		{m.groups_title()}
		<select class={field} bind:value={group}>
			<option value="any">{m.access_any()}</option>
			{#each groups as g (g.sid)}
				<option value={g.sid}>{g.name}</option>
			{/each}
		</select>
	</label>
	{#if narrowed}
		<button type="button" class="rounded-lg px-2 py-1.5 text-sm hover:bg-surface-2" onclick={clear}>
			{m.access_clear_filters()}
		</button>
	{/if}
	<span class="ml-auto text-sm text-ink-2 tabular-nums">
		{m.access_count({ shown: filtered.length, total: users.length })}
	</span>
	{#if admin && local}
		<button
			type="button"
			class="inline-flex items-center gap-2 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
			onclick={showInvite}
		>
			<UserPlus size={16} aria-hidden="true" />
			{m.users_invite()}
		</button>
	{/if}
</div>

<div class="mt-4 overflow-x-auto rounded-card border border-line bg-surface">
	<table class="w-full text-left text-sm">
		<thead class="border-b border-line text-ink-2">
			<tr>
				<th class="px-4 py-2 font-medium">{m.field_name()}</th>
				<th class="px-4 py-2 font-medium">{m.users_col_source()}</th>
				<th class="px-4 py-2 font-medium">{m.groups_title()}</th>
				<th class="px-4 py-2 font-medium">{m.access_col_role()}</th>
				<th class="px-4 py-2 font-medium">{m.users_col_state()}</th>
				<th class="px-4 py-2 font-medium">{m.users_col_second_factor()}</th>
				<th class="px-4 py-2 font-medium">{m.users_col_last_sign_in()}</th>
			</tr>
		</thead>
		<tbody>
			{#each shown as user (user.id)}
				<tr class="border-b border-line last:border-0" data-testid="user-row">
					<td class={cell}>
						<a
							class="font-medium text-accent hover:underline"
							href={accessHref({ tab: 'users', user: user.id })}>{user.display_name}</a
						>
						{#if user.display_name !== user.username}
							<div class="text-xs text-ink-3">{user.username}</div>
						{/if}
					</td>
					<td class={cell}>
						<span class="rounded-md border border-line px-1.5 py-0.5 text-xs text-ink-2">
							{USER_KIND_LABELS[user.kind]()}
						</span>
					</td>
					<td class={cell}>
						<div class="flex flex-wrap gap-1">
							{#each user.groups.slice(0, 2) as g (g.sid)}
								<span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-xs">{g.name}</span>
							{/each}
							{#if user.groups.length > 2}
								<span class="text-xs text-ink-3"
									>{m.access_more({ count: user.groups.length - 2 })}</span
								>
							{/if}
						</div>
					</td>
					<td class={cell}>
						{#each [...new Set(user.roles.map((r) => r.role))] as role (role)}
							<span class="mr-1 rounded-md bg-surface-2 px-1.5 py-0.5 text-xs">
								{SITE_ROLE_LABELS[role]()}
							</span>
						{/each}
					</td>
					<td class={cell}>
						<span class="inline-flex items-center gap-2">
							{#if user.blocked}
								<Ban size={14} class="text-critical" aria-hidden="true" />
								{m.users_blocked()}
							{:else}
								<CircleCheck size={14} class="text-ok" aria-hidden="true" />
								{m.users_active()}
							{/if}
						</span>
					</td>
					<td class={cell}>
						<span class="inline-flex items-center gap-2 text-ink-2">
							{#if user.second_factor}
								<ShieldCheck size={14} class="text-ok" aria-hidden="true" />
								{m.users_factor_on()}
							{:else}
								<ShieldOff size={14} class="text-ink-3" aria-hidden="true" />
								{m.users_factor_off()}
							{/if}
						</span>
					</td>
					<td class="{cell} whitespace-nowrap text-ink-2 tabular-nums">
						{user.last_sign_in_at
							? time.format(new Date(user.last_sign_in_at))
							: m.connectors_never()}
					</td>
				</tr>
			{:else}
				<tr
					><td colspan="7" class="px-4 py-6 text-center text-ink-2">{m.access_nobody_matches()}</td
					></tr
				>
			{/each}
		</tbody>
	</table>
</div>

{#if pages > 1}
	<nav class="mt-3 flex items-center justify-end gap-2 text-sm" aria-label={m.access_pages()}>
		<button
			type="button"
			class="rounded-lg px-2 py-1 hover:bg-surface-2 disabled:opacity-40"
			disabled={page === 0}
			onclick={() => (page -= 1)}
		>
			{m.access_previous()}
		</button>
		<span class="text-ink-2 tabular-nums">{m.access_page({ page: page + 1, pages })}</span>
		<button
			type="button"
			class="rounded-lg px-2 py-1 hover:bg-surface-2 disabled:opacity-40"
			disabled={page >= pages - 1}
			onclick={() => (page += 1)}
		>
			{m.access_next()}
		</button>
	</nav>
{/if}

<Dialog
	bind:open={dialogOpen}
	title={code ? m.users_code_title({ name: code.name }) : m.users_invite()}
>
	{#if code}
		<CodeView code={code.code} onclose={() => (dialogOpen = false)} />
	{:else}
		<form onsubmit={invite}>
			<p class="text-sm text-ink-2">{m.users_invite_hint()}</p>
			<label class="mt-4 block text-sm font-medium" for="invite-email">{m.sign_in_email()}</label>
			<input
				id="invite-email"
				type="email"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				required
				maxlength="320"
				autocomplete="off"
				bind:value={email}
			/>
			<label class="mt-4 block text-sm font-medium" for="invite-name">{m.field_name()}</label>
			<input
				id="invite-name"
				class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
				maxlength="200"
				autocomplete="off"
				bind:value={name}
			/>
			{#if mailServer}
				<MailChoice bind:sendMail bind:language />
			{/if}
			{#if error}
				<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
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
					disabled={busy}
				>
					{m.users_invite()}
				</button>
			</div>
		</form>
	{/if}
</Dialog>
