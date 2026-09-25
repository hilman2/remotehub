<script lang="ts">
	/**
	 * Before the first remotehub session of a local account (#103): a new
	 * password after an invitation or recovery (`page.state.newPassword`), then the
	 * authenticator app that remotehub requires.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { errorMessage } from '$lib/api/errors';
	import Logo from '$lib/components/Logo.svelte';
	import {
		loadFlow,
		messages,
		startFlow,
		submitFlow,
		type Flow,
		type FlowResult,
		type UiText
	} from '$lib/kratos/flow';
	import Messages from '$lib/kratos/Messages.svelte';
	import TotpSetup from '$lib/kratos/TotpSetup.svelte';
	import { m } from '$lib/paraglide/messages';
	import { signInLocal } from '$lib/session.svelte';

	let flow = $state<Flow | null>(null);
	let needsPassword = $state(page.state.newPassword === true);
	let password = $state('');
	let busy = $state(false);
	let texts = $state<UiText[]>([]);
	let error = $state<string | null>(null);

	// Read once: leaving for the devices page changes `page.state`, and a
	// new flow started then would send the browser back to the sign-in.
	const settingsFlow = page.state.settingsFlow;

	$effect(() => {
		(settingsFlow ? loadFlow('settings', settingsFlow) : startFlow('settings')).then(follow);
	});

	async function follow(result: FlowResult) {
		busy = false;
		switch (result.kind) {
			case 'flow':
				flow = result.flow;
				texts = messages(result.flow);
				if (result.flow.state === 'success' && needsPassword) {
					needsPassword = false;
					texts = [];
				} else if (result.flow.state === 'success') {
					await establish();
				}
				return;
			case 'redirect':
			case 'expired':
				// No Kratos session, or it is too old to change anything.
				await goto(resolve('/sign-in'));
				return;
			default:
				error = errorMessage('accounts_unavailable');
		}
	}

	async function setPassword(event: SubmitEvent) {
		event.preventDefault();
		if (!flow) return;
		busy = true;
		const result = await submitFlow(flow, { method: 'password', password });
		password = '';
		await follow(result);
	}

	async function establish() {
		const result = await signInLocal();
		if (result.ok) await goto(resolve('/'));
		else error = errorMessage(result.code);
	}

	const field = 'h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5';
</script>

<section class="flex min-h-dvh items-center justify-center px-6 py-16">
	<div class="flex w-full max-w-sm flex-col gap-5">
		<div class="mb-2 flex items-center gap-3">
			<Logo size={28} />
			<span class="font-display text-lg font-bold">remotehub</span>
		</div>
		<h1 class="text-3xl font-semibold">{m.setup_title()}</h1>

		{#if flow && needsPassword}
			<form class="flex flex-col gap-5" onsubmit={setPassword}>
				<p class="text-sm text-ink-2">{m.setup_password_hint()}</p>
				<div class="flex flex-col gap-2">
					<label class="text-sm font-medium" for="new-password">{m.account_new_password()}</label>
					<input
						id="new-password"
						type="password"
						class={field}
						autocomplete="new-password"
						minlength="12"
						required
						bind:value={password}
					/>
				</div>
				<Messages {texts} />
				<button
					type="submit"
					disabled={busy}
					class="h-13 w-full rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
				>
					{m.setup_continue()}
				</button>
			</form>
		{:else if flow}
			<p class="text-sm text-ink-2">{m.setup_totp_hint()}</p>
			<Messages {texts} />
			<TotpSetup {flow} ondone={follow} />
		{/if}

		{#if error}
			<p class="flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{error}
			</p>
		{/if}
	</div>
</section>
