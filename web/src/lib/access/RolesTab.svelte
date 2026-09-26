<script lang="ts">
	/**
	 * Roles for remotehub itself (#178): one card per role with whom it is
	 * given to, and everyone who holds a role, with the way it reaches them.
	 */
	import type { AccessUser } from '$lib/api/access';
	import { m } from '$lib/paraglide/messages';
	import Roles from '$lib/users/Roles.svelte';
	import { SITE_ROLE_LABELS } from './labels';
	import { accessHref } from './links';

	let { users, admin, onchanged }: { users: AccessUser[]; admin: boolean; onchanged: () => void } =
		$props();

	const holders = $derived(
		users.flatMap((user) => user.roles.map((role) => ({ user, role: role.role, via: role.via })))
	);
</script>

<Roles {admin} {onchanged} />

<section class="mt-8" aria-labelledby="role-holders">
	<h2 id="role-holders" class="text-lg font-semibold">{m.access_role_holders()}</h2>
	<div class="mt-3 overflow-x-auto rounded-card border border-line bg-surface">
		<table class="w-full text-left text-sm">
			<thead class="border-b border-line text-ink-2">
				<tr>
					<th class="px-4 py-2 font-medium">{m.field_name()}</th>
					<th class="px-4 py-2 font-medium">{m.access_col_role()}</th>
					<th class="px-4 py-2 font-medium">{m.permissions_through()}</th>
				</tr>
			</thead>
			<tbody>
				{#each holders as h (`${h.user.id}${h.role}${h.via.sid}`)}
					<tr class="border-b border-line last:border-0">
						<td class="px-4 py-2">
							<a
								class="text-accent hover:underline"
								href={accessHref({ tab: 'users', user: h.user.id })}>{h.user.display_name}</a
							>
						</td>
						<td class="px-4 py-2">{SITE_ROLE_LABELS[h.role]()}</td>
						<td class="px-4 py-2 text-ink-2">
							{h.via.sid === h.user.principal
								? m.access_role_own()
								: m.access_role_via({ name: h.via.name })}
						</td>
					</tr>
				{:else}
					<tr><td colspan="3" class="px-4 py-6 text-center text-ink-2">{m.roles_none()}</td></tr>
				{/each}
			</tbody>
		</table>
	</div>
	<p class="mt-2 text-xs text-ink-3">{m.access_role_holders_note()}</p>
</section>
