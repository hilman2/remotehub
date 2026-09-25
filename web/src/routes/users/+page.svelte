<script lang="ts">
	import Ban from '@lucide/svelte/icons/ban';
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import UserPlus from '@lucide/svelte/icons/user-plus';
	import { errorMessage } from '$lib/api/errors';
	import {
		blockUser,
		deleteUser,
		endSessions,
		inviteUser,
		issueRecovery,
		loadUsers,
		unblockUser,
		type OneTimeCode,
		type UserKind,
		type UserRow
	} from '$lib/api/users';
	import Dialog from '$lib/components/Dialog.svelte';
	import Groups from '$lib/users/Groups.svelte';
	import SettingsMenu, { type MenuItem } from '$lib/components/SettingsMenu.svelte';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { loadMethods, session } from '$lib/session.svelte';

	/** A change that waits for the administrator's confirmation. */
	type Change = 'block' | 'unblock' | 'sessions' | 'recovery' | 'delete';

	type Open =
		| { type: 'invite' }
		| { type: 'confirm'; change: Change; user: UserRow }
		| { type: 'code'; code: OneTimeCode; name: string };

	let users = $state<UserRow[]>([]);
	let local = $state(false);
	let error = $state<string | null>(null);
	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let dialogError = $state<string | null>(null);
	let email = $state('');
	let name = $state('');
	let busy = $state(false);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	const kinds: Record<UserKind, () => string> = {
		directory: m.users_kind_directory,
		local: m.users_kind_local,
		break_glass: m.users_kind_break_glass
	};

	async function load() {
		const result = await loadUsers();
		if (result.ok) users = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		if (!session.user?.admin) return;
		load();
		loadMethods().then((result) => (local = result.ok && result.data.local));
	});

	function show(next: Open) {
		dialogError = null;
		email = '';
		name = '';
		open = next;
		dialogOpen = true;
	}

	/** The signed-in administrator; the server refuses to block or delete them. */
	const isSelf = (user: UserRow) =>
		user.username === session.user?.username && user.kind === session.user?.kind;

	function menu(user: UserRow): MenuItem[] {
		const confirm = (change: Change) => () => show({ type: 'confirm', change, user });
		const items: MenuItem[] = [];
		if (user.sessions > 0 && !isSelf(user)) {
			items.push({ label: m.users_end_sessions(), icon: LogOut, onselect: confirm('sessions') });
		}
		if (user.kind === 'local') {
			items.push({ label: m.users_recovery(), icon: KeyRound, onselect: confirm('recovery') });
		}
		if (user.blocked) {
			items.push({ label: m.users_unblock(), icon: CircleCheck, onselect: confirm('unblock') });
		} else if (!isSelf(user)) {
			items.push({ label: m.users_block(), icon: Ban, danger: true, onselect: confirm('block') });
		}
		if (user.kind === 'local' && !isSelf(user)) {
			items.push({
				label: m.catalog_delete(),
				icon: Trash2,
				danger: true,
				onselect: confirm('delete')
			});
		}
		return items;
	}

	async function invite(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		const result = await inviteUser(email, name);
		busy = false;
		if (!result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		open = { type: 'code', code: result.data, name: name.trim() || email.trim() };
		await load();
	}

	async function apply(change: Change, user: UserRow) {
		busy = true;
		const result = await {
			block: blockUser,
			unblock: unblockUser,
			sessions: endSessions,
			recovery: issueRecovery,
			delete: deleteUser
		}[change](user.id);
		busy = false;
		if (!result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		if (change === 'recovery') {
			open = { type: 'code', code: result.data as OneTimeCode, name: user.display_name };
		} else {
			dialogOpen = false;
		}
		await load();
	}

	const questions: Record<Change, (name: string) => string> = {
		block: (name) => m.users_block_confirm({ name }),
		unblock: (name) => m.users_unblock_confirm({ name }),
		sessions: (name) => m.users_end_sessions_confirm({ name }),
		recovery: (name) => m.users_recovery_confirm({ name }),
		delete: (name) => m.users_delete_confirm({ name })
	};

	const actions: Record<Change, () => string> = {
		block: m.users_block,
		unblock: m.users_unblock,
		sessions: m.users_end_sessions,
		recovery: m.users_recovery,
		delete: m.catalog_delete
	};

	const title = $derived.by(() => {
		switch (open?.type) {
			case 'invite':
				return m.users_invite();
			case 'confirm':
				return actions[open.change]();
			case 'code':
				return m.users_code_title({ name: open.name });
			default:
				return '';
		}
	});
</script>

<div class="flex flex-wrap items-start justify-between gap-4">
	<h1 class="text-4xl font-semibold">{m.users_title()}</h1>
	{#if session.user?.admin && local}
		<button
			type="button"
			class="inline-flex items-center gap-2 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
			onclick={() => show({ type: 'invite' })}
		>
			<UserPlus size={16} aria-hidden="true" />
			{m.users_invite()}
		</button>
	{/if}
</div>

{#if !session.user?.admin}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else if error}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{:else}
	<p class="mt-2 text-sm text-ink-2">{m.users_hint()}</p>
	<div class="mt-6 overflow-x-auto rounded-card border border-line bg-surface">
		<table class="w-full text-left text-sm">
			<thead class="border-b border-line text-ink-2">
				<tr>
					<th class="px-4 py-2 font-medium">{m.field_name()}</th>
					<th class="px-4 py-2 font-medium">{m.users_col_source()}</th>
					<th class="px-4 py-2 font-medium">{m.users_col_state()}</th>
					<th class="px-4 py-2 font-medium">{m.users_col_last_sign_in()}</th>
					<th class="px-4 py-2 font-medium">{m.users_col_sessions()}</th>
					<th class="px-4 py-2"><span class="sr-only">{m.users_actions()}</span></th>
				</tr>
			</thead>
			<tbody>
				{#each users as user (user.id)}
					<tr class="border-b border-line last:border-0" data-testid="user-row">
						<td class="px-4 py-2">
							<div class="font-medium">{user.display_name}</div>
							{#if user.display_name !== user.username}
								<div class="text-xs text-ink-3">{user.username}</div>
							{/if}
						</td>
						<td class="px-4 py-2">
							<span class="rounded-md border border-line px-1.5 py-0.5 text-xs text-ink-2">
								{kinds[user.kind]()}
							</span>
						</td>
						<td class="px-4 py-2">
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
						<td class="px-4 py-2 whitespace-nowrap text-ink-2 tabular-nums">
							{user.last_sign_in_at
								? time.format(new Date(user.last_sign_in_at))
								: m.connectors_never()}
						</td>
						<td class="px-4 py-2 text-ink-2 tabular-nums">{user.sessions}</td>
						<td class="px-4 py-2">
							<div class="flex justify-end">
								<SettingsMenu
									label={m.users_actions_for({ name: user.display_name })}
									items={menu(user)}
								/>
							</div>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
	<Groups />
{/if}

<Dialog bind:open={dialogOpen} {title}>
	{#if open?.type === 'invite'}
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
					disabled={busy}
				>
					{m.users_invite()}
				</button>
			</div>
		</form>
	{:else if open?.type === 'confirm'}
		{@const { change, user } = open}
		<p class="text-sm">{questions[change](user.display_name)}</p>
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
				onclick={() => apply(change, user)}
			>
				{actions[change]()}
			</button>
		</div>
	{:else if open?.type === 'code'}
		{@const code = open.code}
		<p class="text-sm">{m.users_code_hint()}</p>
		<dl class="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
			<dt class="text-ink-2">{m.users_code_link()}</dt>
			<dd class="font-mono text-xs break-all select-all" data-testid="code-link">{code.link}</dd>
			<dt class="text-ink-2">{m.sign_in_code()}</dt>
			<dd class="font-mono select-all" data-testid="code-value">{code.code}</dd>
			<dt class="text-ink-2">{m.users_code_expires()}</dt>
			<dd class="tabular-nums">{time.format(new Date(code.expires_at))}</dd>
		</dl>
		<div class="mt-5 flex justify-end">
			<button
				type="button"
				class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
				onclick={() => (dialogOpen = false)}
			>
				{m.action_close()}
			</button>
		</div>
	{/if}
</Dialog>
