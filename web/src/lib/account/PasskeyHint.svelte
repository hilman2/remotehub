<script lang="ts">
	/**
	 * Points someone without a passkey to setting one up (#244): unlocking a
	 * session and confirming take one touch with it, where the authenticator
	 * app wants a code. Shown until dismissed here or a passkey is set up.
	 */
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import X from '@lucide/svelte/icons/x';
	import { resolve } from '$app/paths';
	import { api } from '$lib/api/client';
	import { webauthnAvailable } from '$lib/kratos/webauthn';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';

	const KEY = 'remotehub.passkey-hint';

	let shown = $state(false);

	function dismissed(): boolean {
		try {
			return localStorage.getItem(KEY) === 'dismissed';
		} catch {
			return false;
		}
	}

	$effect(() => {
		if (session.user?.kind === 'break_glass' || !webauthnAvailable() || dismissed()) return;
		let current = true;
		api<{ app: boolean; keys: boolean }>('GET', '/api/session/factors').then((factors) => {
			if (current && factors.ok) shown = !factors.data.keys;
		});
		return () => {
			current = false;
		};
	});

	function dismiss() {
		shown = false;
		try {
			localStorage.setItem(KEY, 'dismissed');
		} catch {
			// Shown again next time, then.
		}
	}
</script>

{#if shown}
	<div
		class="flex items-start gap-3 rounded-card border border-line bg-surface p-4 text-sm"
		role="note"
	>
		<Fingerprint size={18} class="mt-0.5 shrink-0 text-accent" aria-hidden="true" />
		<p class="flex-1">
			{m.confirm_passkey_hint()}
			<a href={resolve('/account')} class="font-medium underline">{m.confirm_passkey_set_up()}</a>
		</p>
		<button
			type="button"
			class="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink"
			title={m.passkey_hint_dismiss()}
			onclick={dismiss}
		>
			<X size={16} aria-hidden="true" />
			<span class="sr-only">{m.passkey_hint_dismiss()}</span>
		</button>
	</div>
{/if}
