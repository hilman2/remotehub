<script lang="ts">
	/**
	 * Shows or copies a stored password or key for someone with `reveal`
	 * (#97). The server records each time. Shown values hide again after a
	 * while, and a copied one leaves the clipboard.
	 */
	import Copy from '@lucide/svelte/icons/copy';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import { errorMessage } from '$lib/api/errors';
	import { reveal, type RevealPurpose, type Revealed } from '$lib/api/reveal';
	import { m } from '$lib/paraglide/messages';

	let { owner, id }: { owner: 'credentials' | 'devices'; id: string } = $props();

	/** How long a shown or copied value stays. */
	const SECONDS = 30;

	let shown = $state<Revealed | null>(null);
	let copied = $state(false);
	let error = $state<string | null>(null);
	let hideTimer: ReturnType<typeof setTimeout> | undefined;

	async function fetchSecret(purpose: RevealPurpose) {
		error = null;
		const result = await reveal(owner, id, purpose);
		if (!result.ok) {
			error = errorMessage(result.code);
			return null;
		}
		return result.data;
	}

	async function show() {
		shown = await fetchSecret('show');
		clearTimeout(hideTimer);
		hideTimer = setTimeout(() => (shown = null), SECONDS * 1000);
	}

	function hide() {
		clearTimeout(hideTimer);
		shown = null;
	}

	async function copy() {
		const secret = await fetchSecret('copy');
		const value = secret?.password ?? secret?.private_key;
		if (!value) return;
		try {
			await navigator.clipboard.writeText(value);
		} catch {
			error = m.reveal_copy_failed();
			return;
		}
		copied = true;
		setTimeout(async () => {
			copied = false;
			// Only what is still the copied value goes. A browser that does not
			// let the page read the clipboard gets it cleared all the same.
			let current: string;
			try {
				current = await navigator.clipboard.readText();
			} catch {
				current = value;
			}
			if (current === value) await navigator.clipboard.writeText('').catch(() => {});
		}, SECONDS * 1000);
	}

	$effect(() => () => clearTimeout(hideTimer));

	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line-strong px-2.5 py-1 text-sm hover:bg-surface-2';
</script>

<div class="flex flex-col gap-2">
	<div class="flex flex-wrap gap-2">
		{#if shown}
			<button type="button" class={button} onclick={hide}>
				<EyeOff size={14} aria-hidden="true" />
				{m.reveal_hide()}
			</button>
		{:else}
			<button type="button" class={button} onclick={show}>
				<Eye size={14} aria-hidden="true" />
				{m.reveal_show()}
			</button>
		{/if}
		<button type="button" class={button} onclick={copy}>
			<Copy size={14} aria-hidden="true" />
			{m.reveal_copy()}
		</button>
	</div>
	{#if copied}
		<p class="text-xs text-ink-2" role="status">{m.reveal_copied({ seconds: SECONDS })}</p>
	{/if}
	{#if shown}
		{#if shown.password !== undefined}
			<p class="font-mono text-sm break-all select-all" data-testid="revealed">{shown.password}</p>
		{/if}
		{#if shown.private_key !== undefined}
			<pre
				class="max-h-48 overflow-auto rounded-lg border border-line bg-page p-2 font-mono text-xs select-all"
				data-testid="revealed">{shown.private_key}</pre>
			{#if shown.passphrase}
				<p class="text-xs text-ink-2">
					{m.field_passphrase()}:
					<span class="font-mono select-all">{shown.passphrase}</span>
				</p>
			{/if}
		{/if}
		{#each shown.fields ?? [] as field (field.name)}
			<p class="text-xs text-ink-2">
				{field.name}:
				<span class="font-mono break-all select-all" data-testid="revealed-field"
					>{field.value}</span
				>
			</p>
		{/each}
	{/if}
	{#if error}
		<p class="text-sm text-critical" role="alert">{error}</p>
	{/if}
</div>
