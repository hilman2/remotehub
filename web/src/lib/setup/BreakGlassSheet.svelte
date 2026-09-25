<script lang="ts">
	/**
	 * A break-glass account as a sheet to print and keep in a safe (#143):
	 * where it signs in, its name, its password, and its TOTP key as text and
	 * as a QR code for an authenticator app.
	 */
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import type { BreakGlassAccount } from '$lib/api/setup';
	import QrCode from '$lib/components/QrCode.svelte';
	import { m } from '$lib/paraglide/messages';

	let { account }: { account: BreakGlassAccount } = $props();

	const signIn = $derived(new URL(resolve('/sign-in/break-glass'), page.url.origin).href);
</script>

<div class="print-sheet rounded-card border border-line bg-surface p-6">
	<h2 class="text-lg font-semibold">{m.wizard_break_glass_sheet()}</h2>
	<p class="mt-1 text-sm text-ink-2">{m.wizard_break_glass_sheet_hint()}</p>
	<dl class="mt-4 grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 text-sm">
		<dt class="text-ink-2">{m.wizard_break_glass_address()}</dt>
		<dd class="font-mono break-all">{signIn}</dd>
		<dt class="text-ink-2">{m.sign_in_username()}</dt>
		<dd class="font-mono" data-testid="break-glass-username">{account.username}</dd>
		<dt class="text-ink-2">{m.sign_in_password()}</dt>
		<dd class="font-mono break-all select-all" data-testid="break-glass-password">
			{account.password}
		</dd>
		<dt class="text-ink-2">{m.account_totp_secret()}</dt>
		<dd class="font-mono break-all select-all" data-testid="break-glass-totp">
			{account.totp_secret}
		</dd>
	</dl>
	<div class="mt-4">
		<QrCode value={account.totp_uri} label={m.account_totp_qr()} />
	</div>
</div>
