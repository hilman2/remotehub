<script lang="ts">
	/**
	 * Makes an organisation recovery key (#95): a passphrase for its file,
	 * then the private key once, as the downloaded file and as text to
	 * print. Used on the vault recovery page and in the setup wizard (#143).
	 */
	import Printer from '@lucide/svelte/icons/printer';
	import { problemMessage } from '$lib/api/errors';
	import { m } from '$lib/paraglide/messages';
	import { createKey } from '$lib/vault/recovery';

	let {
		oncreated,
		ondone
	}: {
		/** The key exists now; its private key is on the screen. */
		oncreated?: () => void;
		/** The owner has put the private key away. */
		ondone: () => void;
	} = $props();

	/** As long as a vault's passphrase must be. */
	const MIN_PASSPHRASE = 12;

	let passphrase = $state('');
	let passphraseAgain = $state('');
	let error = $state<string | null>(null);
	let privateText = $state<string | null>(null);
	let busy = $state(false);

	async function make(event: SubmitEvent) {
		event.preventDefault();
		if (passphrase.length < MIN_PASSPHRASE) {
			error = m.vault_passphrase_short({ count: MIN_PASSPHRASE });
			return;
		}
		if (passphrase !== passphraseAgain) {
			error = m.vault_passphrase_mismatch();
			return;
		}
		busy = true;
		const made = await createKey(passphrase);
		busy = false;
		if (!made.ok) {
			error = problemMessage(made);
			return;
		}
		error = null;
		passphrase = passphraseAgain = '';
		privateText = made.data.text;
		const blob = new Blob([JSON.stringify(made.data.file, null, 2)], {
			type: 'application/json'
		});
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.href = url;
		link.download = `remotehub-recovery-key-${made.data.file.key_id.slice(0, 8)}.json`;
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
		oncreated?.();
	}

	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

{#if privateText}
	<div class="print-sheet">
		<p class="text-sm">{m.recovery_key_shown_once()}</p>
		<p
			class="mt-4 rounded-lg bg-surface-2 p-3 font-mono text-lg break-all"
			data-testid="organisation-private-key"
		>
			{privateText}
		</p>
	</div>
	<div class="mt-5 flex flex-wrap gap-2 print:hidden">
		<button type="button" class={button} onclick={() => window.print()}>
			<Printer size={14} aria-hidden="true" />
			{m.print()}
		</button>
		<button type="button" class={primary} onclick={ondone}>
			{m.vault_recovery_saved()}
		</button>
	</div>
{:else}
	<form onsubmit={make}>
		<p class="text-sm text-ink-2">{m.recovery_key_create_hint()}</p>
		<label class={label} for="recovery-new-passphrase">{m.recovery_key_passphrase()}</label>
		<input
			id="recovery-new-passphrase"
			class={field}
			type="password"
			required
			autocomplete="new-password"
			bind:value={passphrase}
		/>
		<label class={label} for="recovery-new-passphrase-again">
			{m.field_passphrase_again()}
		</label>
		<input
			id="recovery-new-passphrase-again"
			class={field}
			type="password"
			required
			autocomplete="new-password"
			bind:value={passphraseAgain}
		/>
		<button type="submit" class="{primary} mt-5" disabled={busy}>
			{m.recovery_key_create()}
		</button>
		{#if error}
			<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
		{/if}
	</form>
{/if}
