<script lang="ts">
	/** The current code of a TOTP field (#100) and how long it holds. */
	import { m } from '$lib/paraglide/messages';
	import { totpCode, type TotpParams } from './totp';

	let { params }: { params: TotpParams } = $props();

	let code = $state('');
	let left = $state(0);

	$effect(() => {
		let stopped = false;
		const tick = async () => {
			const now = Date.now() / 1000;
			left = Math.ceil(params.period - (now % params.period));
			const next = await totpCode(params, now);
			if (!stopped) code = next;
		};
		tick();
		const timer = setInterval(tick, 1000);
		return () => {
			stopped = true;
			clearInterval(timer);
		};
	});
</script>

<span class="inline-flex items-baseline gap-2">
	<span class="font-mono tracking-widest select-all" data-testid="totp-code">{code}</span>
	<span class="text-xs text-ink-3">{m.vault_totp_left({ seconds: left })}</span>
</span>
