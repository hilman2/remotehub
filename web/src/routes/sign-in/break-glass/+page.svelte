<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Siren from '@lucide/svelte/icons/siren';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { errorMessage } from '$lib/api/errors';
	import { m } from '$lib/paraglide/messages';
	import { signInBreakGlass } from '$lib/session.svelte';

	let username = $state('');
	let password = $state('');
	let code = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		const result = await signInBreakGlass(username, password, code);
		busy = false;
		if (result.ok) {
			password = '';
			code = '';
			await goto(resolve('/'));
		} else {
			code = '';
			error = errorMessage(result.code);
		}
	}
</script>

<div class="flex min-h-dvh items-center justify-center px-4">
	<form
		class="w-full max-w-sm rounded-card border border-line bg-surface p-8 shadow-card"
		onsubmit={submit}
	>
		<div class="mb-6 flex flex-col items-center gap-3 text-center">
			<Siren size={36} class="text-critical" aria-hidden="true" />
			<h1 class="text-xl font-semibold tracking-tight">{m.break_glass_title()}</h1>
			<p class="text-sm text-ink-2">{m.break_glass_intro()}</p>
		</div>

		<label class="block text-sm font-medium" for="username">{m.sign_in_username()}</label>
		<input
			id="username"
			name="username"
			autocomplete="username"
			required
			bind:value={username}
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
		/>

		<label class="mt-4 block text-sm font-medium" for="password">{m.sign_in_password()}</label>
		<input
			id="password"
			name="password"
			type="password"
			autocomplete="current-password"
			required
			bind:value={password}
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
		/>

		<label class="mt-4 block text-sm font-medium" for="code">{m.break_glass_code()}</label>
		<input
			id="code"
			name="code"
			inputmode="numeric"
			autocomplete="one-time-code"
			pattern="[0-9]*"
			maxlength="6"
			required
			bind:value={code}
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2 font-mono tracking-widest"
		/>

		{#if error}
			<p class="mt-4 flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{error}
			</p>
		{/if}

		<button
			type="submit"
			disabled={busy}
			class="mt-6 w-full rounded-lg bg-accent px-4 py-2 font-medium text-accent-ink disabled:opacity-60"
		>
			{busy ? m.sign_in_busy() : m.sign_in_submit()}
		</button>

		<a
			href={resolve('/sign-in')}
			class="mt-4 block text-center text-sm text-ink-2 hover:text-ink hover:underline"
		>
			{m.break_glass_back()}
		</a>
	</form>
</div>
