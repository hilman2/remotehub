<script lang="ts">
	/** Settings of the instance, for administrators: who states a purpose (#90). */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import type { Principal } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import {
		loadPurposePrincipals,
		requirePurpose,
		waivePurpose,
		type PurposePrincipal
	} from '$lib/api/journal';
	import PrincipalName from '$lib/catalog/PrincipalName.svelte';
	import PrincipalPicker from '$lib/catalog/PrincipalPicker.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';

	let principals = $state<PurposePrincipal[]>([]);
	let chosen = $state<Principal | null>(null);
	/** Counts the principals added here: a new picker for each. */
	let added = $state(0);
	let error = $state<string | null>(null);

	async function load() {
		const result = await loadPurposePrincipals();
		if (result.ok) principals = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		if (session.user?.admin) load();
	});

	async function add(event: SubmitEvent) {
		event.preventDefault();
		if (!chosen) return;
		const result = await requirePurpose(chosen);
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		chosen = null;
		added += 1;
		error = null;
		await load();
	}

	async function remove(sid: string) {
		const result = await waivePurpose(sid);
		if (result.ok) await load();
		else error = errorMessage(result.code);
	}
</script>

<h1 class="text-4xl font-semibold">{m.nav_settings()}</h1>

{#if !session.user?.admin}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else}
	<section class="mt-8 max-w-2xl rounded-card border border-line bg-surface p-6">
		<h2 class="text-xl font-semibold">{m.purpose_rules_title()}</h2>
		<p class="mt-1 text-sm text-ink-2">{m.purpose_rules_hint()}</p>

		{#if principals.length === 0}
			<p class="mt-4 text-sm text-ink-3">{m.purpose_rules_none()}</p>
		{:else}
			<ul
				class="mt-4 divide-y divide-line rounded-lg border border-line"
				aria-label={m.purpose_rules_title()}
			>
				{#each principals as p (p.principal_sid)}
					<li class="flex items-center gap-3 px-3 py-2 text-sm">
						<PrincipalName kind={p.principal_kind} name={p.principal_name} />
						<button
							type="button"
							class="ml-auto rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
							onclick={() => remove(p.principal_sid)}
						>
							{m.grants_remove()}
						</button>
					</li>
				{/each}
			</ul>
		{/if}

		<form class="mt-5 border-t border-line pt-4" onsubmit={add}>
			{#key added}
				<PrincipalPicker id="purpose-search" bind:chosen onerror={(message) => (error = message)} />
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
{/if}
