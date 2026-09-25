<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Clock from '@lucide/svelte/icons/clock';
	import {
		ROLES,
		addGrant,
		loadGrants,
		removeGrant,
		type GrantRow,
		type ObjectKind,
		type Principal,
		type Role
	} from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { ROLE_LABELS } from './labels';
	import PrincipalName from './PrincipalName.svelte';
	import PrincipalPicker from './PrincipalPicker.svelte';

	let { kind, id }: { kind: ObjectKind; id: string } = $props();

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	let direct = $state<GrantRow[]>([]);
	let inherited = $state<GrantRow[]>([]);
	let chosen = $state<Principal | null>(null);
	/** Counts the grants added here: a new picker for each. */
	let added = $state(0);
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

	async function grant(event: SubmitEvent) {
		event.preventDefault();
		if (!chosen) return;
		const result = await addGrant(kind, id, chosen, role);
		if (result.ok) {
			chosen = null;
			added += 1;
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

{#snippet until(expiresAt: string | null)}
	{#if expiresAt}
		<span class="inline-flex items-center gap-1 text-xs text-ink-3">
			<Clock size={13} aria-hidden="true" />
			{m.grants_until({ time: time.format(new Date(expiresAt)) })}
		</span>
	{/if}
{/snippet}

<h3 class="text-sm font-medium text-ink-2">{m.grants_direct()}</h3>
{#if direct.length === 0}
	<p class="mt-2 text-sm text-ink-3">{m.grants_none()}</p>
{:else}
	<ul class="mt-2 divide-y divide-line rounded-lg border border-line">
		{#each direct as g (g.id)}
			<li class="flex items-center gap-3 px-3 py-2 text-sm">
				<PrincipalName kind={g.principal_kind} name={g.principal_name} />
				<span class="ml-auto text-ink-2">{ROLE_LABELS[g.role]()}</span>
				{@render until(g.expires_at)}
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
				<PrincipalName kind={g.principal_kind} name={g.principal_name} />
				<span class="ml-auto">{ROLE_LABELS[g.role]()}</span>
				{@render until(g.expires_at)}
			</li>
		{/each}
	</ul>
{/if}

<form class="mt-5 border-t border-line pt-4" onsubmit={grant}>
	{#key added}
		<PrincipalPicker id="grant-search" bind:chosen onerror={(message) => (error = message)} />
	{/key}

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
