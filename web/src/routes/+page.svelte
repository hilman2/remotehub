<script lang="ts">
	import MonitorSmartphone from '@lucide/svelte/icons/monitor-smartphone';
	import { m } from '$lib/paraglide/messages';
</script>

<h1 class="text-2xl font-semibold tracking-tight">{m.devices_title()}</h1>

<section
	class="mt-8 flex flex-col items-center gap-3 rounded-card border border-dashed border-line bg-surface px-6 py-16 text-center"
>
	<MonitorSmartphone size={40} class="text-ink-3" aria-hidden="true" />
	<h2 class="text-lg font-medium">{m.devices_empty_title()}</h2>
	<p class="max-w-md text-sm text-ink-2">{m.devices_empty_text()}</p>
</section>
