<script lang="ts">
	/**
	 * Who has which role for remotehub itself (#106). The administrators
	 * configured at installation are not listed: they cannot be removed here.
	 */
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import type { Principal } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { assignRole, loadRoles, revokeRole, type RoleAssignments } from '$lib/api/roles';
	import PrincipalName from '$lib/catalog/PrincipalName.svelte';
	import PrincipalPicker from '$lib/catalog/PrincipalPicker.svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import { m } from '$lib/paraglide/messages';
	import type { Role } from '$lib/session.svelte';

	let roles = $state<RoleAssignments[]>([]);
	let error = $state<string | null>(null);
	let adding = $state<Role | null>(null);
	let dialogOpen = $state(false);
	let dialogError = $state<string | null>(null);
	let chosen = $state<Principal | null>(null);

	const names: Record<Role, () => string> = {
		administrator: m.role_administrator,
		auditor: m.role_auditor,
		security_officer: m.role_security_officer
	};
	const hints: Record<Role, () => string> = {
		administrator: m.role_administrator_hint,
		auditor: m.role_auditor_hint,
		security_officer: m.role_security_officer_hint
	};

	async function load() {
		const result = await loadRoles();
		if (result.ok) roles = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		load();
	});

	function add(role: Role) {
		adding = role;
		chosen = null;
		dialogError = null;
		dialogOpen = true;
	}

	async function assign(event: SubmitEvent) {
		event.preventDefault();
		if (!adding || !chosen) return;
		const result = await assignRole(adding, chosen);
		if (!result.ok) {
			dialogError = errorMessage(result.code);
			return;
		}
		dialogOpen = false;
		await load();
	}

	async function revoke(role: Role, sid: string) {
		const result = await revokeRole(role, sid);
		if (!result.ok) error = errorMessage(result.code);
		await load();
	}
</script>

<section class="mt-12" aria-labelledby="roles-title">
	<h2 id="roles-title" class="text-2xl font-semibold">{m.roles_title()}</h2>
	<p class="mt-2 text-sm text-ink-2">{m.roles_hint()}</p>
	{#if error}
		<p class="mt-4 text-sm text-critical" role="alert">{error}</p>
	{/if}
	<div class="mt-4 grid gap-4 md:grid-cols-3">
		{#each roles as assignments (assignments.role)}
			{@const role = assignments.role}
			<div class="rounded-card border border-line bg-surface p-4" data-testid="role-{role}">
				<div class="flex items-start justify-between gap-2">
					<h3 class="font-medium">{names[role]()}</h3>
					<button
						type="button"
						class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
						title={m.roles_add({ role: names[role]() })}
						onclick={() => add(role)}
					>
						<Plus size={15} aria-hidden="true" />
						<span class="sr-only">{m.roles_add({ role: names[role]() })}</span>
					</button>
				</div>
				<p class="mt-1 text-xs text-ink-3">{hints[role]()}</p>
				{#if assignments.members.length === 0}
					<p class="mt-3 text-sm text-ink-2">{m.roles_none()}</p>
				{:else}
					<ul class="mt-3 space-y-1">
						{#each assignments.members as member (member.sid)}
							<li class="flex items-center gap-2 text-sm">
								<PrincipalName kind={member.kind} name={member.name} />
								<button
									type="button"
									class="ml-auto rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
									title={m.groups_remove_member({ name: member.name })}
									onclick={() => revoke(role, member.sid)}
								>
									<X size={14} aria-hidden="true" />
									<span class="sr-only">{m.groups_remove_member({ name: member.name })}</span>
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
		{/each}
	</div>
</section>

<Dialog bind:open={dialogOpen} title={adding ? m.roles_add({ role: names[adding]() }) : ''}>
	<form onsubmit={assign}>
		{#key dialogOpen}
			<PrincipalPicker
				id="role-member"
				bind:chosen
				onerror={(message) => (dialogError = message)}
			/>
		{/key}
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
				class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50"
				disabled={!chosen}
			>
				{m.groups_add_member()}
			</button>
		</div>
	</form>
</Dialog>
