<script lang="ts">
	/**
	 * The authenticator app of a directory user (#107): set it up, or remove
	 * it with one of its codes unless a rule asks for it.
	 */
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { errorMessage } from '$lib/api/errors';
	import {
		enrollFactor,
		loadFactor,
		offerFactor,
		removeFactor,
		type FactorOffer,
		type FactorStatus
	} from '$lib/api/secondFactor';
	import AuthenticatorSetup from '$lib/components/AuthenticatorSetup.svelte';
	import { m } from '$lib/paraglide/messages';

	let status = $state<FactorStatus | null>(null);
	let offer = $state<FactorOffer | null>(null);
	let code = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	async function load() {
		const result = await loadFactor();
		if (result.ok) status = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		load();
	});

	async function begin() {
		error = null;
		const result = await offerFactor();
		if (result.ok) offer = result.data;
		else error = errorMessage(result.code);
	}

	async function done(result: { ok: boolean; code?: string }) {
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		error = null;
		offer = null;
		await load();
	}

	async function enroll(typed: string) {
		if (!offer) return;
		busy = true;
		await done(await enrollFactor(offer.secret, typed));
	}

	async function remove(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		const typed = code.trim();
		code = '';
		await done(await removeFactor(typed));
	}

	const button =
		'self-start rounded-xl border border-line-strong bg-surface px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-60';
</script>

<section class="flex max-w-3xl flex-col gap-3 rounded-card border border-line bg-surface p-6">
	<h2 class="text-lg font-semibold">{m.account_totp()}</h2>
	{#if status?.enrolled}
		<p class="flex items-center gap-2 text-sm">
			<CircleCheck size={16} class="text-ok" aria-hidden="true" />
			{m.account_totp_active()}
		</p>
		{#if status.required}
			<p class="text-sm text-ink-2">{m.factor_required_here()}</p>
		{:else}
			<form class="flex flex-wrap items-end gap-3" onsubmit={remove}>
				<div class="flex flex-col gap-1">
					<label class="text-sm font-medium" for="factor-remove-code">
						{m.account_totp_code()}
					</label>
					<input
						id="factor-remove-code"
						class="h-11 w-40 rounded-xl border border-line-strong bg-page px-3 font-mono tracking-widest"
						inputmode="numeric"
						autocomplete="one-time-code"
						required
						bind:value={code}
					/>
				</div>
				<button type="submit" class={button} disabled={busy}>{m.factor_remove()}</button>
			</form>
		{/if}
	{:else if offer}
		<AuthenticatorSetup secret={offer.secret} uri={offer.uri} {busy} onconfirm={enroll} />
	{:else if status}
		<p class="text-sm text-ink-2">{m.factor_none()}</p>
		<button type="button" class={button} onclick={begin}>{m.account_totp_confirm()}</button>
	{/if}
	{#if error}
		<p class="text-sm text-critical" role="alert">{error}</p>
	{/if}
</section>
