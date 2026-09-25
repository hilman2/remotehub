<script lang="ts">
	/** A session alone in its own window, beside the tabs inside remotehub. */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import { page } from '$app/state';
	import { loadTree, type Device } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import SessionView from '$lib/session/SessionView.svelte';

	let device = $state<Device | null>(null);
	let askPurpose = $state(false);
	let error = $state<string | null>(null);

	$effect(() => {
		const id = page.params.id;
		loadTree().then((result) => {
			if (!result.ok) {
				error = errorMessage(result.code);
				return;
			}
			askPurpose = result.data.purpose_required;
			device = result.data.devices.find((d) => d.id === id) ?? null;
			if (!device) error = errorMessage('not_found');
		});
	});

	$effect(() => {
		if (device) document.title = `${device.name} · remotehub`;
	});
</script>

{#if error}
	<p class="flex items-center gap-2 px-6 py-10 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{:else if device}
	<h1 class="sr-only">{device.name}</h1>
	<div class="flex min-h-0 flex-1 flex-col">
		<SessionView {device} {askPurpose} />
	</div>
{/if}
