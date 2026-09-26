<script lang="ts">
	/**
	 * Whether the customer lets remotehub reach this device now (#180), as a
	 * chip on the device's page. Asks every minute: the customer may open or
	 * close it at any time. Shows nothing while the connector cannot say.
	 */
	import Lock from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import { loadConnectorAccess, type ConnectorAccess } from '$lib/api/connectors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let { deviceId }: { deviceId: string } = $props();

	const EVERY_MS = 60 * 1000;

	let access = $state<ConnectorAccess | null>(null);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	$effect(() => {
		const id = deviceId;
		let current = true;
		access = null;
		async function read() {
			const result = await loadConnectorAccess(id);
			if (current) access = result.ok ? result.data : null;
		}
		read();
		const reading = setInterval(read, EVERY_MS);
		return () => {
			current = false;
			clearInterval(reading);
		};
	});
</script>

{#if access?.state === 'open'}
	<span class="chip">
		<LockOpen size={13} class="text-ok" aria-hidden="true" />
		<span>
			{access.until
				? m.device_access_open_until({ until: time.format(new Date(access.until)) })
				: m.device_access_open()}
		</span>
	</span>
{:else if access?.state === 'closed'}
	<span class="chip">
		<Lock size={13} class="text-ink-2" aria-hidden="true" />
		<span>{m.device_access_closed()}</span>
	</span>
{/if}
