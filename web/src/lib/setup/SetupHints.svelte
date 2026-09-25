<script lang="ts">
	/**
	 * What the setup wizard (#143) left out, for administrators: a
	 * break-glass account and the organisation recovery key. Each hint stays
	 * until it is made up for or dismissed in this browser.
	 */
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { resolve } from '$app/paths';
	import { loadUsers } from '$lib/api/users';
	import { m } from '$lib/paraglide/messages';
	import { loadKeys } from '$lib/vault/recovery';

	type Hint = 'break_glass' | 'recovery_key';

	const STORE = 'remotehub.setup-hints.dismissed';

	function dismissed(): Hint[] {
		try {
			return JSON.parse(localStorage.getItem(STORE) ?? '[]') as Hint[];
		} catch {
			return [];
		}
	}

	let missing = $state<Hint[]>([]);
	let hidden = $state<Hint[]>(dismissed());

	$effect(() => {
		Promise.all([loadUsers(), loadKeys()]).then(([users, keys]) => {
			const found: Hint[] = [];
			if (users.ok && !users.data.some((user) => user.kind === 'break_glass')) {
				found.push('break_glass');
			}
			if (keys.ok && keys.data.keys.length === 0) found.push('recovery_key');
			missing = found;
		});
	});

	function dismiss(hint: Hint) {
		hidden = [...hidden, hint];
		try {
			localStorage.setItem(STORE, JSON.stringify(hidden));
		} catch {
			// Without storage, the hint only goes for this page.
		}
	}

	const shown = $derived(missing.filter((hint) => !hidden.includes(hint)));
</script>

{#each shown as hint (hint)}
	<div
		class="flex items-center justify-center gap-3 border-b border-line bg-surface px-4 py-2 text-sm"
		role="status"
	>
		<TriangleAlert size={16} class="text-warning" aria-hidden="true" />
		{#if hint === 'break_glass'}
			<span>{m.wizard_hint_break_glass()}</span>
		{:else}
			<span>{m.wizard_hint_recovery_key()}</span>
			<a class="text-accent hover:underline" href={resolve('/recovery')}>{m.nav_recovery()}</a>
		{/if}
		<button type="button" class="text-ink-2 hover:underline" onclick={() => dismiss(hint)}>
			{m.wizard_hint_dismiss()}
		</button>
	</div>
{/each}
