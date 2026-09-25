<script lang="ts">
	/**
	 * Setting up an authenticator app with a key remotehub offers (#107): the
	 * key to type or open in the app, then a code from the app to confirm.
	 */
	import { m } from '$lib/paraglide/messages';

	let {
		secret,
		uri,
		busy = false,
		onconfirm
	}: {
		secret: string;
		/** `otpauth://…`, which opens the app on a phone. */
		uri: string;
		busy?: boolean;
		onconfirm: (code: string) => void;
	} = $props();

	/** A code of an authenticator app. */
	const SIX_DIGITS = '[0-9]{6}';

	let code = $state('');

	function confirm(event: SubmitEvent) {
		event.preventDefault();
		onconfirm(code.trim());
		code = '';
	}
</script>

<form class="flex flex-col gap-3" onsubmit={confirm}>
	<p class="text-sm text-ink-2">{m.factor_setup_hint()}</p>
	<p class="text-xs text-ink-3">
		{m.account_totp_secret()}
		<span class="font-mono break-all text-ink-2 select-all" data-testid="totp-secret">{secret}</span
		>
	</p>
	<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- an app's URI, not a page -->
	<a class="self-start text-sm text-accent hover:underline" href={uri}>{m.factor_open_app()}</a>
	<label class="text-sm font-medium" for="factor-setup-code">{m.account_totp_code()}</label>
	<input
		id="factor-setup-code"
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
