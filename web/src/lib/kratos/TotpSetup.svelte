<script lang="ts">
	/**
	 * Setting up an authenticator app in a Kratos settings flow: the QR code
	 * and the secret Kratos offers, then a code from the app to confirm it.
	 */
	import { m } from '$lib/paraglide/messages';
	import { node, submitFlow, type Flow, type FlowResult } from './flow';

	let { flow, ondone }: { flow: Flow; ondone: (result: FlowResult) => void } = $props();

	/** A code of an authenticator app. */
	const SIX_DIGITS = '[0-9]{6}';

	let code = $state('');
	let busy = $state(false);

	const qr = $derived(node(flow, 'totp_qr')?.attributes.src);
	const secret = $derived(node(flow, 'totp_secret_key')?.attributes.text?.text);

	async function confirm(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		const result = await submitFlow(flow, { method: 'totp', totp_code: code.trim() });
		busy = false;
		code = '';
		ondone(result);
	}
</script>

<form class="flex flex-col gap-3" onsubmit={confirm}>
	<p class="text-sm text-ink-2">{m.account_totp_hint()}</p>
	{#if qr}
		<img src={qr} alt={m.account_totp_qr()} class="size-44 self-start rounded-lg bg-white p-2" />
	{/if}
	{#if secret}
		<p class="text-xs text-ink-3">
			{m.account_totp_secret()}
			<span class="font-mono break-all text-ink-2 select-all" data-testid="totp-secret"
				>{secret}</span
			>
		</p>
	{/if}
	<label class="text-sm font-medium" for="totp-setup-code">{m.account_totp_code()}</label>
	<input
		id="totp-setup-code"
		class="h-11 w-40 rounded-xl border border-line-strong bg-page px-3 font-mono tracking-widest"
		inputmode="numeric"
		autocomplete="one-time-code"
		pattern={SIX_DIGITS}
		required
		bind:value={code}
	/>
	<button
		type="submit"
		disabled={busy}
		class="self-start rounded-xl bg-accent px-4 py-2 text-sm font-semibold text-accent-ink disabled:opacity-60"
	>
		{m.account_totp_confirm()}
	</button>
</form>
