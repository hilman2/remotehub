<script lang="ts">
	/**
	 * An invitation or a forgotten password (#103): the one-time code from the
	 * administrator or the e-mail opens the account for a new password.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { errorMessage } from '$lib/api/errors';
	import Logo from '$lib/components/Logo.svelte';
	import {
		flowId,
		loadFlow,
		messages,
		offers,
		startFlow,
		submitFlow,
		type Flow,
		type FlowResult,
		type UiText
	} from '$lib/kratos/flow';
	import Messages from '$lib/kratos/Messages.svelte';
	import { m } from '$lib/paraglide/messages';

	let flow = $state<Flow | null>(null);
	let email = $state('');
	let code = $state('');
	let busy = $state(false);
	let texts = $state<UiText[]>([]);
	let error = $state<string | null>(null);

	/** Kratos asks for the address first, then for the code sent to it. */
	const asksCode = $derived(flow !== null && offers(flow, 'code'));

	$effect(() => {
		const id = page.url.searchParams.get('flow') ?? page.state.recoveryFlow;
		(id ? loadFlow('recovery', id) : startFlow('recovery')).then(follow);
	});

	async function follow(result: FlowResult) {
		busy = false;
		switch (result.kind) {
			case 'flow':
				flow = result.flow;
				texts = messages(result.flow);
				return;
			case 'redirect': {
				const settings = flowId(result.to);
				if (result.to.pathname.endsWith('/settings') || result.to.pathname.endsWith('/account')) {
					await goto(resolve('/sign-in/setup'), {
						state: { settingsFlow: settings ?? undefined, newPassword: true }
					});
				} else {
					// The account has a second factor: it signs in with it.
					await goto(resolve('/sign-in'), { state: { secondFactor: true } });
				}
				return;
			}
			case 'expired': {
				// A new flow asks for the address again.
				error = m.kratos_expired();
				const again = await startFlow('recovery');
				flow = again.kind === 'flow' ? again.flow : null;
				return;
			}
			default:
				error = errorMessage('accounts_unavailable');
		}
	}

	// The setup wizard (#143) hands over the code of the administrator it
	// just invited: nobody types it.
	let handed = page.state.recoveryCode;
	$effect(() => {
		if (!handed || !asksCode || busy) return;
		code = handed;
		handed = undefined;
		send();
	});

	function submit(event: SubmitEvent) {
		event.preventDefault();
		send();
	}

	async function send() {
		if (!flow) return;
		busy = true;
		error = null;
		const values = asksCode
			? { method: 'code', code: code.trim() }
			: { method: 'code', email: email.trim() };
		code = '';
		await follow(await submitFlow(flow, values));
	}

	const field = 'h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5';
</script>

<section class="flex min-h-dvh items-center justify-center px-6 py-16">
	<form class="flex w-full max-w-sm flex-col gap-5" onsubmit={submit}>
		<div class="mb-2 flex items-center gap-3">
			<Logo size={28} />
			<span class="font-display text-lg font-bold">remotehub</span>
		</div>
		<h1 class="text-3xl font-semibold">{m.recovery_title()}</h1>
		<p class="text-sm text-ink-2">{asksCode ? m.recovery_code_hint() : m.recovery_email_hint()}</p>

		{#if asksCode}
			<div class="flex flex-col gap-2">
				<label class="text-sm font-medium" for="recovery-code">{m.sign_in_code()}</label>
				<input
					id="recovery-code"
					class="{field} font-mono tracking-widest"
					inputmode="numeric"
					autocomplete="one-time-code"
					required
					bind:value={code}
				/>
			</div>
		{:else}
			<div class="flex flex-col gap-2">
				<label class="text-sm font-medium" for="recovery-email">{m.sign_in_email()}</label>
				<input
					id="recovery-email"
					type="email"
					class={field}
					autocomplete="email"
					required
					bind:value={email}
				/>
			</div>
		{/if}

		<Messages {texts} />
		{#if error}
			<p class="flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{error}
			</p>
		{/if}

		<button
			type="submit"
			disabled={busy || !flow}
			class="h-13 w-full rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
		>
			{m.recovery_continue()}
		</button>
		<a
			href={resolve('/sign-in')}
			class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
		>
			{m.recovery_back()}
		</a>
	</form>
</section>
