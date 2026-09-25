<script lang="ts">
	/**
	 * Finds a user or group in the directory while typing, from two
	 * characters on, and sets `chosen` to the one picked. Typing again clears
	 * the choice; to start over, mount it anew (`{#key}`).
	 */
	import { searchPrincipals, type Principal } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import { m } from '$lib/paraglide/messages';
	import PrincipalName from './PrincipalName.svelte';

	let {
		id,
		chosen = $bindable(null),
		onerror,
		accepts = () => true
	}: {
		id: string;
		chosen?: Principal | null;
		onerror: (message: string) => void;
		/** Which of the found principals may be chosen here. */
		accepts?: (principal: Principal) => boolean;
	} = $props();

	let query = $state('');
	let found = $state<Principal[]>([]);

	$effect(() => {
		const q = query.trim();
		if (q.length < 2 || chosen) {
			found = [];
			return;
		}
		const timer = setTimeout(async () => {
			const result = await searchPrincipals(q);
			found = result.ok ? result.data.filter(accepts) : [];
			if (!result.ok) onerror(errorMessage(result.code));
		}, 250);
		return () => clearTimeout(timer);
	});
</script>

<label class="block text-sm font-medium" for={id}>{m.grants_search()}</label>
<input
	{id}
	class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
	autocomplete="off"
	bind:value={query}
	oninput={() => (chosen = null)}
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
					<PrincipalName kind={p.kind} name={p.name} />
					{#if p.detail}<span class="ml-auto truncate text-xs text-ink-3">{p.detail}</span>{/if}
				</button>
			</li>
		{/each}
	</ul>
{/if}
