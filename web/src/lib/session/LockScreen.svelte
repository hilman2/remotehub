<script lang="ts">
	/**
	 * Covers the page while the session is locked (#241): a modal dialog, so
	 * nothing behind it takes a key or a click, and Escape does not close
	 * it. Pages and connections behind it stay as they are.
	 */
	import Lock from '@lucide/svelte/icons/lock';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Logo from '$lib/components/Logo.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';
	import ConfirmFactor from './ConfirmFactor.svelte';
	import { confirmed } from './confirm.svelte';

	let { onsignout }: { onsignout: () => void } = $props();

	let dialog: HTMLDialogElement;

	$effect(() => {
		if (!dialog.open) dialog.showModal();
	});

	function unlocked() {
		if (session.user) session.user.locked = false;
		confirmed(true);
	}
</script>

<dialog
	bind:this={dialog}
	class="m-0 flex h-dvh max-h-none w-screen max-w-none items-center justify-center bg-page p-4 text-ink backdrop:bg-page"
	aria-labelledby="lock-title"
	oncancel={(event) => event.preventDefault()}
>
	<div class="w-full max-w-md rounded-card border border-line bg-surface p-7 shadow-float">
		<div class="flex items-center gap-2.5">
			<Logo />
			<span class="font-display text-lg font-bold tracking-tight">remotehub</span>
		</div>
		<h2 id="lock-title" class="mt-6 flex items-center gap-2 text-2xl font-semibold">
			<Lock size={22} aria-hidden="true" />
			{m.lock_title()}
		</h2>
		<p class="mt-1 mb-5 text-sm text-ink-2">
			{m.lock_hint({ name: session.user?.display_name ?? '' })}
		</p>
		<ConfirmFactor ondone={unlocked} />
		<button
			type="button"
			class="mt-5 inline-flex items-center gap-1.5 text-sm text-ink-3 hover:text-ink hover:underline"
			onclick={onsignout}
		>
			<LogOut size={15} aria-hidden="true" />
			{m.sign_out()}
		</button>
	</div>
</dialog>
