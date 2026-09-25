<script lang="ts">
	/**
	 * What the setup wizard (#143) left out, for administrators: a
	 * break-glass account and the organisation recovery key; and what the
	 * certificate (#146) asks for: the root certificate of Caddy's own CA on
	 * the clients, a certificate of your own before it runs out. Each hint
	 * stays until it is made up for or dismissed in this browser; the one
	 * about running out is dismissed per certificate.
	 */
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { resolve } from '$app/paths';
	import { loadCertificate, runsOutSoon } from '$lib/api/certificate';
	import { loadUsers } from '$lib/api/users';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { loadKeys } from '$lib/vault/recovery';

	type Hint =
		| { kind: 'break_glass' | 'recovery_key' | 'certificate_root'; key: string }
		| { kind: 'certificate_expiry'; key: string; date: string };

	const STORE = 'remotehub.setup-hints.dismissed';

	function dismissed(): string[] {
		try {
			return JSON.parse(localStorage.getItem(STORE) ?? '[]') as string[];
		} catch {
			return [];
		}
	}

	let missing = $state<Hint[]>([]);
	let hidden = $state<string[]>(dismissed());

	const date = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'long' });

	$effect(() => {
		Promise.all([loadUsers(), loadKeys(), loadCertificate()]).then(([users, keys, certificate]) => {
			const found: Hint[] = [];
			if (users.ok && !users.data.some((user) => user.kind === 'break_glass')) {
				found.push({ kind: 'break_glass', key: 'break_glass' });
			}
			if (keys.ok && keys.data.keys.length === 0) {
				found.push({ kind: 'recovery_key', key: 'recovery_key' });
			}
			if (certificate.ok && certificate.data.source === 'remotehub') {
				found.push({ kind: 'certificate_root', key: 'certificate_root' });
			}
			const own = certificate.ok ? certificate.data.own : undefined;
			if (own && runsOutSoon(own.info)) {
				found.push({
					kind: 'certificate_expiry',
					key: `certificate_expiry:${own.info.fingerprint}`,
					date: date.format(new Date(own.info.not_after * 1000))
				});
			}
			missing = found;
		});
	});

	function dismiss(hint: Hint) {
		hidden = [...hidden, hint.key];
		try {
			localStorage.setItem(STORE, JSON.stringify(hidden));
		} catch {
			// Without storage, the hint only goes for this page.
		}
	}

	const shown = $derived(missing.filter((hint) => !hidden.includes(hint.key)));
	const link = 'text-accent hover:underline';
</script>

{#each shown as hint (hint.key)}
	<div
		class="flex items-center justify-center gap-3 border-b border-line bg-surface px-4 py-2 text-sm"
		role="status"
	>
		<TriangleAlert size={16} class="text-warning" aria-hidden="true" />
		{#if hint.kind === 'break_glass'}
			<span>{m.wizard_hint_break_glass()}</span>
		{:else if hint.kind === 'recovery_key'}
			<span>{m.wizard_hint_recovery_key()}</span>
			<a class={link} href={resolve('/recovery')}>{m.nav_recovery()}</a>
		{:else}
			<span>
				{hint.kind === 'certificate_expiry'
					? m.wizard_hint_certificate_expiry({ date: hint.date })
					: m.wizard_hint_certificate_root()}
			</span>
			<a class={link} href="{resolve('/settings')}#certificate">{m.certificate_title()}</a>
		{/if}
		<button type="button" class="text-ink-2 hover:underline" onclick={() => dismiss(hint)}>
			{m.wizard_hint_dismiss()}
		</button>
	</div>
{/each}
