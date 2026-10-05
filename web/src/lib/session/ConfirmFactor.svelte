<script lang="ts">
	/**
	 * Confirming with the second factor (#241, #242): a passkey or security
	 * key first, one touch; the authenticator app's code otherwise. Who has
	 * only the app is pointed to a passkey (#244).
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import Lightbulb from '@lucide/svelte/icons/lightbulb';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import { resolve } from '$app/paths';
	import { errorMessage } from '$lib/api/errors';
	import { getCredential, webauthnAvailable } from '$lib/kratos/webauthn';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';
	import { sendConfirmation, startConfirmation, type ConfirmStart } from './confirm.svelte';

	let {
		ondone,
		onaccount
	}: {
		/** Confirmed. */
		ondone: () => void;
		/** Leaves for the account page, where a factor is set up. */
		onaccount?: () => void;
	} = $props();

	let start = $state<ConfirmStart | null>(null);
	let code = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	const keys = $derived(!!start?.key && webauthnAvailable());
	/** Break-glass accounts have their code alone. */
	const mayAddKey = $derived(session.user?.kind !== 'break_glass' && webauthnAvailable());

	/** When `start` came: its challenge lives five minutes on the server. */
	let loadedAt = 0;
	const stale = () => Date.now() - loadedAt >= FRESH_MS;

	async function load() {
		const result = await startConfirmation();
		if (result.ok) {
			start = result.data;
			loadedAt = Date.now();
		} else error = errorMessage(result.code);
	}

	// The lock screen waits while nobody is there: a challenge loaded when
	// it came would have expired by the time they are back (#253).
	$effect(() => {
		load();
		const timer = setInterval(() => {
			if (!busy && stale()) load();
		}, CHECK_MS);
		return () => clearInterval(timer);
	});

	async function withKey() {
		busy = true;
		error = null;
		try {
			// Timers stand still while the computer sleeps.
			if (stale()) await load();
			if (!start?.key) return;
			let credential: unknown;
			try {
				credential = JSON.parse(await getCredential(JSON.stringify(start.key.options)));
			} catch {
				// Cancelled or no answer: the challenge is still unused.
				error = m.passkey_failed();
				return;
			}
			const result = await sendConfirmation({
				key: { challenge_id: start.key.challenge_id, credential }
			});
			if (result.ok) {
				ondone();
				return;
			}
			error = m.confirm_key_refused();
			// A refused answer used its challenge up.
			await load();
		} finally {
			busy = false;
		}
	}

	async function withCode(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		const result = await sendConfirmation({ code: code.trim() });
		busy = false;
		if (result.ok) {
			ondone();
			return;
		}
		code = '';
		error = errorMessage(result.code);
	}

	/** Renewed a minute before the server's five minutes run out. */
	const FRESH_MS = 4 * 60_000;
	const CHECK_MS = 30_000;

	const field = 'h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5';
</script>

{#if start}
	<div class="flex flex-col gap-4">
		{#if keys}
			<button
				type="button"
				disabled={busy}
				class="inline-flex h-13 w-full items-center justify-center gap-2 rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
				onclick={withKey}
			>
				<Fingerprint size={20} aria-hidden="true" />
				{m.confirm_with_passkey()}
			</button>
		{/if}
		{#if start.app}
			<form class="flex flex-col gap-3" onsubmit={withCode}>
				<label class="text-sm font-medium" for="confirm-code">
					{keys ? m.confirm_or_code() : m.confirm_code()}
				</label>
				<div class="flex gap-2">
					<input
						id="confirm-code"
						class="{field} font-mono tracking-widest"
						autocomplete="one-time-code"
						inputmode="numeric"
						required
						bind:value={code}
					/>
					<button
						type="submit"
						disabled={busy}
						class="inline-flex h-12 shrink-0 items-center gap-2 rounded-xl border border-line-strong bg-surface px-4 font-medium hover:bg-surface-2 disabled:opacity-60"
					>
						<ShieldCheck size={18} aria-hidden="true" />
						{m.confirm_submit()}
					</button>
				</div>
			</form>
		{/if}
		{#if !start.app && !start.key}
			<p class="flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{m.confirm_no_factor()}
			</p>
		{/if}
		{#if error}
			<p class="flex items-start gap-2 text-sm" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
				{error}
			</p>
		{/if}
		{#if !start.key && mayAddKey}
			<!-- #244: a passkey is one touch where the app is a code to type. -->
			<div class="flex items-start gap-2.5 rounded-xl border border-line bg-surface-2 p-3 text-sm">
				<Lightbulb size={16} class="mt-0.5 shrink-0 text-accent" aria-hidden="true" />
				<p>
					{m.confirm_passkey_hint()}
					{#if onaccount}
						<a href={resolve('/account')} class="font-medium underline" onclick={onaccount}>
							{m.confirm_passkey_set_up()}
						</a>
					{/if}
				</p>
			</div>
		{/if}
	</div>
{:else if error}
	<p class="flex items-start gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
		{error}
	</p>
{/if}
