<script lang="ts">
	/** A user or a group by name, with an icon that tells which. */
	import User from '@lucide/svelte/icons/user';
	import Users from '@lucide/svelte/icons/users';
	import { m } from '$lib/paraglide/messages';

	let { kind, name }: { kind: 'user' | 'group'; name: string } = $props();
</script>

<span class="inline-flex min-w-0 items-center gap-2">
	{#if kind === 'group'}
		<Users size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
		<span class="sr-only">{m.principal_group()}</span>
	{:else}
		<User size={15} class="shrink-0 text-ink-3" aria-hidden="true" />
		<span class="sr-only">{m.principal_user()}</span>
	{/if}
	<span class="truncate">{name}</span>
</span>
