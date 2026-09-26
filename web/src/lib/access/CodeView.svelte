<script lang="ts">
	/** A one-time sign-in code for a local account, shown this once. */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import type { OneTimeCode } from '$lib/api/users';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let { code, onclose }: { code: OneTimeCode; onclose: () => void } = $props();

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
</script>

<p class="text-sm">{m.users_code_hint()}</p>
<dl class="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
	<dt class="text-ink-2">{m.users_code_link()}</dt>
	<dd class="font-mono text-xs break-all select-all" data-testid="code-link">{code.link}</dd>
	<dt class="text-ink-2">{m.sign_in_code()}</dt>
	<dd class="font-mono select-all" data-testid="code-value">{code.code}</dd>
	<dt class="text-ink-2">{m.users_code_expires()}</dt>
	<dd class="tabular-nums">{time.format(new Date(code.expires_at))}</dd>
</dl>
{#if code.mailed}
	<p class="mt-4 flex items-center gap-2 text-sm" role="status" data-testid="code-mailed">
		<CircleCheck size={16} class="text-ok" aria-hidden="true" />
		{m.users_mailed()}
	</p>
{:else if code.mail_failure}
	<p class="mt-4 flex items-start gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
		<span>
			{m.users_mail_failed()}
			<span class="block font-mono text-xs break-all text-ink-3">{code.mail_failure.detail}</span>
		</span>
	</p>
{/if}
<div class="mt-5 flex justify-end">
	<button
		type="button"
		class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
		onclick={onclose}
	>
		{m.action_close()}
	</button>
</div>
