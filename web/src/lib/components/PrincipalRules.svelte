<script module lang="ts">
	export interface Rule {
		principal_sid: string;
		principal_kind: 'user' | 'group';
		principal_name: string;
	}
</script>

<script lang="ts">
	/**
	 * A list of users and groups a rule of the instance names, with a search
	 * to add one: who states a purpose (#90), who needs a second factor (#107).
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import type { Principal } from '$lib/api/catalog';
	import type { ApiResult } from '$lib/api/client';
	import { errorMessage } from '$lib/api/errors';
	import PrincipalName from '$lib/catalog/PrincipalName.svelte';
	import PrincipalPicker from '$lib/catalog/PrincipalPicker.svelte';
	import { m } from '$lib/paraglide/messages';

	let {
		id,
		title,
		hint,
		load,
		add,
		remove
	}: {
		id: string;
		title: string;
		hint: string;
		load: () => Promise<ApiResult<Rule[]>>;
		add: (principal: Principal) => Promise<ApiResult<unknown>>;
		remove: (sid: string) => Promise<ApiResult<unknown>>;
	} = $props();

	let rules = $state<Rule[]>([]);
	let chosen = $state<Principal | null>(null);
	/** Counts the principals added here: a new picker for each. */
	let added = $state(0);
	let error = $state<string | null>(null);

	async function refresh() {
		const result = await load();
		if (result.ok) rules = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		refresh();
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (!chosen) return;
		const result = await add(chosen);
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		chosen = null;
		added += 1;
		error = null;
		await refresh();
	}

	async function drop(sid: string) {
		const result = await remove(sid);
		if (result.ok) await refresh();
		else error = errorMessage(result.code);
	}
</script>

<section class="mt-8 max-w-2xl rounded-card border border-line bg-surface p-6">
	<h2 class="text-xl font-semibold">{title}</h2>
	<p class="mt-1 text-sm text-ink-2">{hint}</p>

	{#if rules.length === 0}
		<p class="mt-4 text-sm text-ink-3">{m.purpose_rules_none()}</p>
	{:else}
		<ul class="mt-4 divide-y divide-line rounded-lg border border-line" aria-label={title}>
			{#each rules as p (p.principal_sid)}
				<li class="flex items-center gap-3 px-3 py-2 text-sm">
					<PrincipalName kind={p.principal_kind} name={p.principal_name} />
					<button
						type="button"
						class="ml-auto rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
						onclick={() => drop(p.principal_sid)}
					>
						{m.grants_remove()}
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<form class="mt-5 border-t border-line pt-4" onsubmit={submit}>
		{#key added}
			<PrincipalPicker {id} bind:chosen onerror={(message) => (error = message)} />
		{/key}
		<button
			type="submit"
			disabled={!chosen}
			class="mt-3 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-accent-ink disabled:opacity-50"
		>
			{m.purpose_rules_add()}
		</button>
	</form>

	{#if error}
		<p class="mt-3 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
</section>
