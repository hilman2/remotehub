<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import LogIn from '@lucide/svelte/icons/log-in';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import favicon from '$lib/assets/favicon.svg';
	import { errorMessage } from '$lib/api/errors';
	import { m } from '$lib/paraglide/messages';
	import { signIn } from '$lib/session.svelte';

	let username = $state('');
	let password = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		const result = await signIn(username, password);
		busy = false;
		if (result.ok) {
			password = '';
			await goto(resolve('/'));
		} else {
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
			<img src={favicon} alt="" class="size-10" />
			<h1 class="text-xl font-semibold tracking-tight">{m.sign_in_title()}</h1>
			<p class="text-sm text-ink-2">{m.sign_in_intro()}</p>
		</div>

		<label class="block text-sm font-medium" for="username">{m.sign_in_username()}</label>
		<input
			id="username"
			name="username"
			autocomplete="username"
			required
			bind:value={username}
			aria-describedby="username-hint"
			class="mt-1 w-full rounded-lg border border-line bg-page px-3 py-2"
		/>
		<p id="username-hint" class="mt-1 text-xs text-ink-3">{m.sign_in_username_hint()}</p>

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

		{#if error}
			<p class="mt-4 flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{error}
			</p>
		{/if}

		<button
			type="submit"
			disabled={busy}
			class="mt-6 inline-flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-2 font-medium text-accent-ink disabled:opacity-60"
		>
			<LogIn size={16} aria-hidden="true" />
			{busy ? m.sign_in_busy() : m.sign_in_submit()}
		</button>
	</form>
</div>
