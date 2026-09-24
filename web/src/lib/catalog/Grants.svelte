<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import User from '@lucide/svelte/icons/user';
	import Users from '@lucide/svelte/icons/users';
	import {
		ROLES,
		addGrant,
		loadGrants,
		removeGrant,
		searchPrincipals,
		type GrantRow,
		type ObjectKind,
		type Principal,
		type Role
	} from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { m } from '$lib/paraglide/messages';
	import { ROLE_LABELS } from './labels';

	let { kind, id }: { kind: ObjectKind; id: string } = $props();

	let direct = $state<GrantRow[]>([]);
	let inherited = $state<GrantRow[]>([]);
	let query = $state('');
	let found = $state<Principal[]>([]);
	let chosen = $state<Principal | null>(null);
	let role = $state<Role>('connect');
	let error = $state<string | null>(null);

	async function load() {
		const result = await loadGrants(kind, id);
		if (result.ok) {
			direct = result.data.direct;
			inherited = result.data.inherited;
		} else {
			error = errorMessage(result.code);
		}
	}

	$effect(() => {
		load();
	});

	// Searches the directory while typing, from two characters on.
	$effect(() => {
		const q = query.trim();
		if (q.length < 2) {
			found = [];
			return;
		}
		const timer = setTimeout(async () => {
			const result = await searchPrincipals(q);
			found = result.ok ? result.data : [];
			if (!result.ok) error = errorMessage(result.code);
		}, 250);
		return () => clearTimeout(timer);
	});

	async function grant(event: SubmitEvent) {
		event.preventDefault();
		if (!chosen) return;
		const result = await addGrant(kind, id, chosen, role);
		if (result.ok) {
			chosen = null;
			query = '';
			error = null;
			await load();
		} else {
			error = errorMessage(result.code);
		}
	}

	async function remove(grantId: string) {
		const result = await removeGrant(grantId);
		if (result.ok) await load();
		else error = errorMessage(result.code);
	}
</script>

{#snippet principal(kindOf: 'user' | 'group', name: string)}
	<span class="inline-flex min-w-0 items-center gap-2">
		{#if kindOf === 'group'}
			<Users size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
			<span class="sr-only">{m.principal_group()}</span>
		{:else}
			<User size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
			<span class="sr-only">{m.principal_user()}</span>
		{/if}
		<span class="truncate">{name}</span>
	</span>
{/snippet}

<h3 class="text-sm font-medium text-ink-2">{m.grants_direct()}</h3>
{#if direct.length === 0}
	<p class="mt-2 text-sm text-ink-3">{m.grants_none()}</p>
{:else}
	<ul class="mt-2 divide-y divide-line rounded-lg border border-line">
		{#each direct as g (g.id)}
			<li class="flex items-center gap-3 px-3 py-2 text-sm">
				{@render principal(g.principal_kind, g.principal_name)}
				<span class="ml-auto text-ink-2">{ROLE_LABELS[g.role]()}</span>
				<button
					type="button"
					class="rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
					onclick={() => remove(g.id)}
				>
					{m.grants_remove()}
				</button>
			</li>
		{/each}
	</ul>
{/if}

{#if inherited.length > 0}
	<h3 class="mt-4 text-sm font-medium text-ink-2">{m.grants_inherited()}</h3>
	<ul class="mt-2 divide-y divide-line rounded-lg border border-line text-ink-2">
		{#each inherited as g (g.id)}
			<li class="flex items-center gap-3 px-3 py-2 text-sm">
				{@render principal(g.principal_kind, g.principal_name)}
				<span class="ml-auto">{ROLE_LABELS[g.role]()}</span>
			</li>
		{/each}
	</ul>
{/if}

<form class="mt-5 border-t border-line pt-4" onsubmit={grant}>
	<label class="block text-sm font-medium" for="grant-search">{m.grants_search()}</label>
	<input
		id="grant-search"
		class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
		autocomplete="off"
		bind:value={query}
	/>
	{#if found.length > 0 && !chosen}
		<ul class="mt-1 max-h-48 overflow-y-auto rounded-lg border border-line">
			{#each found as p (p.sid)}
				<li>
					<button
						type="button"
						class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-surface-2"
						onclick={() => {
							chosen = p;
							query = p.name;
						}}
					>
						{@render principal(p.kind, p.name)}
						{#if p.detail}<span class="ml-auto truncate text-xs text-ink-3">{p.detail}</span>{/if}
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<div class="mt-3 flex items-center gap-2">
		<select class="flex-1 rounded-lg border border-line bg-page px-3 py-2" bind:value={role}>
			{#each ROLES as r (r)}
				<option value={r}>{ROLE_LABELS[r]()}</option>
			{/each}
		</select>
		<button
			type="submit"
			disabled={!chosen}
			class="rounded-lg bg-accent px-3 py-2 text-sm font-medium text-accent-ink disabled:opacity-50"
		>
			{m.grants_add()}
		</button>
	</div>

	{#if error}
		<p class="mt-3 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
</form>
