<script lang="ts">
	import type { Role } from '$lib/api/catalog';
	import { DURATIONS, requestableRoles } from '$lib/api/requests';
	import { m } from '$lib/paraglide/messages';
	import { ROLE_LABELS, durationLabel } from './labels';

	let {
		held,
		onsubmit,
		oncancel
	}: {
		/** The role the user holds now. */
		held: Role;
		onsubmit: (role: Role, minutes: number, reason: string) => void;
		oncancel: () => void;
	} = $props();

	const roles = $derived(requestableRoles(held));
	// svelte-ignore state_referenced_locally
	let role = $state<Role>(requestableRoles(held)[0] ?? 'connect');
	let minutes = $state(DURATIONS[0]);
	let reason = $state('');

	function submit(event: SubmitEvent) {
		event.preventDefault();
		onsubmit(role, Number(minutes), reason);
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<form onsubmit={submit}>
	<label class="block text-sm font-medium" for="request-role">{m.request_role()}</label>
	<select id="request-role" class={field} bind:value={role}>
		{#each roles as option (option)}
			<option value={option}>{ROLE_LABELS[option]()}</option>
		{/each}
	</select>
	<label class={label} for="request-minutes">{m.request_duration()}</label>
	<select id="request-minutes" class={field} bind:value={minutes}>
		{#each DURATIONS as option (option)}
			<option value={option}>{durationLabel(option)}</option>
		{/each}
	</select>
	<label class={label} for="request-reason">{m.request_reason()}</label>
	<textarea id="request-reason" class={field} rows="3" required maxlength="500" bind:value={reason}
	></textarea>
	<div class="mt-5 flex justify-end gap-2">
		<button
			type="button"
			class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
			onclick={oncancel}
		>
			{m.action_cancel()}
		</button>
		<button
			type="submit"
			class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
		>
			{m.request_send()}
		</button>
	</div>
</form>
