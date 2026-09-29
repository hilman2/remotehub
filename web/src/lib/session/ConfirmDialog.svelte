<script lang="ts">
	/**
	 * Asks for a confirmation with the second factor before a secret is
	 * shown (#242) or a device that asks for it connects (#243). What asked
	 * goes on by itself once it is given.
	 */
	import Dialog from '$lib/components/Dialog.svelte';
	import { m } from '$lib/paraglide/messages';
	import ConfirmFactor from './ConfirmFactor.svelte';
	import { confirmed, confirming, declined } from './confirm.svelte';
</script>

<Dialog
	bind:open={() => confirming.asked, (open) => (open ? undefined : declined())}
	title={m.confirm_title()}
>
	{#if confirming.asked}
		<p class="mb-4 text-sm text-ink-2">{m.confirm_hint()}</p>
		<ConfirmFactor ondone={() => confirmed(true)} onaccount={declined} />
	{/if}
</Dialog>
