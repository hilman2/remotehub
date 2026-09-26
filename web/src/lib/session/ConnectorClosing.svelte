<script lang="ts">
	/**
	 * Warns in a session behind a site connector that the customer's access
	 * ends soon (#165): the connector then ends the connection itself. Reads
	 * the connectors every minute, as the customer may extend or close
	 * access at any time.
	 */
	import Clock from '@lucide/svelte/icons/clock';
	import { loadConnectors } from '$lib/api/connectors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let { connectorId }: { connectorId: string } = $props();

	/** Warn this long before the end. */
	const AHEAD_MS = 10 * 60 * 1000;
	const EVERY_MS = 60 * 1000;

	let name = $state('');
	let until = $state<Date | null>(null);
	let now = $state(Date.now());

	const time = new Intl.DateTimeFormat(formatLocale(), { timeStyle: 'short' });

	$effect(() => {
		const id = connectorId;
		let current = true;
		async function read() {
			const result = await loadConnectors();
			if (!current || !result.ok) return;
			const connector = result.data.find((c) => c.id === id);
			name = connector?.name ?? '';
			until =
				connector?.access === 'open' && connector.open_until
					? new Date(connector.open_until)
					: null;
		}
		read();
		const reading = setInterval(read, EVERY_MS);
		const ticking = setInterval(() => (now = Date.now()), 15_000);
		return () => {
			current = false;
			clearInterval(reading);
			clearInterval(ticking);
		};
	});

	const soon = $derived(until !== null && until.getTime() - now <= AHEAD_MS);
</script>

{#if soon && until}
	<p
		class="absolute top-2 left-1/2 z-10 flex -translate-x-1/2 items-center gap-2 rounded-lg border border-warning/50 bg-surface px-3 py-1.5 text-sm shadow"
		role="status"
	>
		<Clock size={15} class="shrink-0 text-warning" aria-hidden="true" />
		{m.connector_closing({ connector: name, time: time.format(until) })}
	</p>
{/if}
