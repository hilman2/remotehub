<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import LogIn from '@lucide/svelte/icons/log-in';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { PROTOCOLS } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import ProtocolChip from '$lib/catalog/ProtocolChip.svelte';
	import Logo from '$lib/components/Logo.svelte';
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

<div class="grid min-h-dvh lg:grid-cols-2">
	<section
		class="relative hidden flex-col overflow-hidden border-r border-line bg-sunken px-16 py-14 lg:flex"
		style="background-image: linear-gradient(var(--line) 1px, transparent 1px), linear-gradient(90deg, var(--line) 1px, transparent 1px); background-size: 40px 40px"
	>
		<div class="flex items-center gap-3">
			<Logo size={32} />
			<span class="font-display text-xl font-bold">remotehub</span>
		</div>
		<p
			class="mt-auto mb-auto font-display text-6xl leading-[0.98] font-semibold tracking-tight whitespace-pre-line xl:text-7xl"
		>
			{m.sign_in_headline()}
		</p>
		<div class="flex gap-2.5">
			{#each PROTOCOLS as protocol (protocol)}
				<ProtocolChip {protocol} large />
			{/each}
		</div>
	</section>

	<section class="flex items-center justify-center px-6 py-16">
		<form class="flex w-full max-w-sm flex-col gap-5" onsubmit={submit}>
			<div class="mb-2 flex items-center gap-3 lg:hidden">
				<Logo size={28} />
				<span class="font-display text-lg font-bold">remotehub</span>
			</div>
			<h1 class="text-3xl font-semibold">{m.sign_in_title()}</h1>

			<div class="flex flex-col gap-2">
				<label class="text-sm font-medium" for="username">{m.sign_in_username()}</label>
				<input
					id="username"
					name="username"
					autocomplete="username"
					required
					bind:value={username}
					class="h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5"
				/>
			</div>

			<div class="flex flex-col gap-2">
				<label class="text-sm font-medium" for="password">{m.sign_in_password()}</label>
				<input
					id="password"
					name="password"
					type="password"
					autocomplete="current-password"
					required
					bind:value={password}
					class="h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5"
				/>
			</div>

			{#if error}
				<p class="flex items-start gap-2 text-sm" role="alert">
					<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
					{error}
				</p>
			{/if}

			<button
				type="submit"
				disabled={busy}
				class="inline-flex h-13 w-full items-center justify-center gap-2 rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
			>
				<LogIn size={18} aria-hidden="true" />
				{busy ? m.sign_in_busy() : m.sign_in_submit()}
			</button>

			<a
				href={resolve('/sign-in/break-glass')}
				class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
			>
				{m.sign_in_break_glass_link()}
			</a>
		</form>
	</section>
</div>
