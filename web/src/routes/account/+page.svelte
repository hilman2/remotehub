<script lang="ts">
	/**
	 * A local account's own settings (#103): password, authenticator app,
	 * recovery codes and linked providers (#109), through a Kratos settings
	 * flow. Changing them needs a recent sign-in; Kratos sends older sessions
	 * to sign in again.
	 */
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { goto } from '$app/navigation';
	import DirectoryFactor from '$lib/account/DirectoryFactor.svelte';
	import { resolve } from '$app/paths';
	import { errorMessage } from '$lib/api/errors';
	import {
		loadFlow,
		messages,
		node,
		offers,
		providers,
		startFlow,
		submitFlow,
		type Flow,
		type FlowResult,
		type UiText
	} from '$lib/kratos/flow';
	import Messages from '$lib/kratos/Messages.svelte';
	import TotpSetup from '$lib/kratos/TotpSetup.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session, signOut } from '$lib/session.svelte';

	let flow = $state<Flow | null>(null);
	let password = $state('');
	let busy = $state(false);
	let texts = $state<UiText[]>([]);
	let error = $state<string | null>(null);

	const local = $derived(session.user?.kind === 'local');
	const totpSet = $derived(flow !== null && offers(flow, 'totp_unlink'));
	const codes = $derived(flow ? node(flow, 'lookup_secret_codes')?.attributes.text : undefined);

	$effect(() => {
		if (!local) return;
		// Back from a provider (#109), Kratos names the flow with its outcome.
		const id = new URLSearchParams(location.search).get('flow');
		(id ? loadFlow('settings', id) : startFlow('settings')).then(follow);
	});

	const linked = $derived(flow ? providers(flow, 'unlink') : []);
	const linkable = $derived(flow ? providers(flow, 'link') : []);

	/** Links a provider: the browser leaves for it and comes back here. */
	async function link(provider: string) {
		if (!flow) return;
		busy = true;
		error = null;
		const result = await submitFlow(flow, { method: 'oidc', link: provider });
		if (result.kind === 'redirect') {
			location.assign(result.to.href);
			return;
		}
		await follow(result);
	}

	async function follow(result: FlowResult) {
		busy = false;
		switch (result.kind) {
			case 'flow':
				flow = result.flow;
				texts = messages(result.flow);
				return;
			case 'redirect':
			case 'expired':
				// Too long since signing in: remotehub signs out, the user signs
				// in again and comes back.
				await signOut();
				await goto(resolve('/sign-in'));
				return;
			default:
				error = errorMessage('accounts_unavailable');
		}
	}

	async function change(values: Record<string, unknown>) {
		if (!flow) return;
		busy = true;
		error = null;
		await follow(await submitFlow(flow, values));
	}

	async function setPassword(event: SubmitEvent) {
		event.preventDefault();
		const values = { method: 'password', password };
		password = '';
		await change(values);
	}

	const card = 'flex flex-col gap-3 rounded-card border border-line bg-surface p-6';
	const button =
		'self-start rounded-xl border border-line-strong bg-surface px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-60';
</script>

<h1 class="text-4xl font-semibold">{m.account_title()}</h1>

{#if session.user?.kind === 'directory'}
	<p class="mt-2 text-sm text-ink-2">{session.user.username}</p>
	<p class="mt-6 text-sm text-ink-2">{m.account_not_local()}</p>
	<div class="mt-6"><DirectoryFactor /></div>
{:else if !local}
	<p class="mt-6 text-sm text-ink-2">{m.account_not_local()}</p>
{:else if flow}
	<p class="mt-2 text-sm text-ink-2">{session.user?.username}</p>
	<div class="mt-6 flex flex-col gap-2"><Messages {texts} /></div>
	{#if error}
		<p class="mt-4 text-sm" role="alert">{error}</p>
	{/if}

	<div class="mt-6 grid max-w-3xl gap-4">
		<section class={card}>
			<h2 class="text-lg font-semibold">{m.account_password()}</h2>
			<form class="flex flex-wrap items-end gap-3" onsubmit={setPassword}>
				<div class="flex flex-col gap-1">
					<label class="text-sm font-medium" for="account-password">
						{m.account_new_password()}
					</label>
					<input
						id="account-password"
						type="password"
						class="h-11 w-72 rounded-xl border border-line-strong bg-page px-3"
						autocomplete="new-password"
						minlength="12"
						required
						bind:value={password}
					/>
				</div>
				<button type="submit" class={button} disabled={busy}>{m.account_save_password()}</button>
			</form>
		</section>

		<section class={card}>
			<h2 class="text-lg font-semibold">{m.account_totp()}</h2>
			{#if totpSet}
				<p class="flex items-center gap-2 text-sm">
					<CircleCheck size={16} class="text-ok" aria-hidden="true" />
					{m.account_totp_active()}
				</p>
			{:else}
				<TotpSetup {flow} ondone={follow} />
			{/if}
		</section>

		<section class={card}>
			<h2 class="text-lg font-semibold">{m.account_codes()}</h2>
			<p class="text-sm text-ink-2">{m.account_codes_hint()}</p>
			{#if codes && offers(flow, 'lookup_secret_confirm')}
				<pre
					class="rounded-lg border border-line bg-page p-3 font-mono text-sm whitespace-pre-wrap select-all"
					data-testid="recovery-codes">{codes.text}</pre>
				<button
					type="button"
					class={button}
					disabled={busy}
					onclick={() => change({ method: 'lookup_secret', lookup_secret_confirm: true })}
				>
					{m.account_codes_kept()}
				</button>
			{:else if offers(flow, 'lookup_secret_regenerate')}
				<button
					type="button"
					class={button}
					disabled={busy}
					onclick={() => change({ method: 'lookup_secret', lookup_secret_regenerate: true })}
				>
					{m.account_codes_new()}
				</button>
			{/if}
		</section>

		{#if linked.length + linkable.length > 0}
			<section class={card} aria-labelledby="account-providers">
				<h2 id="account-providers" class="text-lg font-semibold">{m.account_providers()}</h2>
				<p class="text-sm text-ink-2">{m.account_providers_hint()}</p>
				<ul class="flex flex-col gap-2">
					{#each linked as provider (provider.id)}
						<li class="flex flex-wrap items-center gap-3 text-sm">
							<CircleCheck size={16} class="text-ok" aria-hidden="true" />
							{m.account_provider_linked({ provider: provider.label })}
							<button
								type="button"
								class={button}
								disabled={busy}
								onclick={() => change({ method: 'oidc', unlink: provider.id })}
							>
								{m.account_provider_unlink({ provider: provider.label })}
							</button>
						</li>
					{/each}
					{#each linkable as provider (provider.id)}
						<li>
							<button
								type="button"
								class={button}
								disabled={busy}
								onclick={() => link(provider.id)}
							>
								{m.account_provider_link({ provider: provider.label })}
							</button>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
	</div>
{/if}
