<script lang="ts">
	/**
	 * Asks the customer for access to a device, or to a folder's devices
	 * behind its site connector (#181), and shows how the latest request of
	 * the signed-in user for it stands. Reads again every 15 seconds while
	 * the customer has not answered.
	 */
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import CircleX from '@lucide/svelte/icons/circle-x';
	import Clock from '@lucide/svelte/icons/clock';
	import Send from '@lucide/svelte/icons/send';
	import type { ObjectKind } from '$lib/api/catalog';
	import {
		askCustomer,
		loadConnectorRequests,
		withdrawConnectorRequest,
		type ConnectorRequest
	} from '$lib/api/connectorRequests';
	import { errorMessage } from '$lib/api/errors';
	import { DURATIONS } from '$lib/api/requests';
	import { durationLabel } from '$lib/catalog/labels';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';

	let { kind, id }: { kind: ObjectKind; id: string } = $props();

	const WHILE_PENDING_MS = 15_000;

	let latest = $state<ConnectorRequest | null>(null);
	let dialogOpen = $state(false);
	let minutes = $state(DURATIONS[0]);
	let reason = $state('');
	let error = $state<string | null>(null);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	async function read() {
		const result = await loadConnectorRequests();
		if (!result.ok) return;
		// An administrator's list holds everyone's; this shows the own.
		latest =
			result.data.find(
				(r) =>
					r.object?.kind === kind &&
					r.object.id === id &&
					r.requester_name === session.user?.username
			) ?? null;
	}

	$effect(() => {
		void [kind, id];
		latest = null;
		read();
	});

	$effect(() => {
		if (latest?.status !== 'pending') return;
		const reading = setInterval(read, WHILE_PENDING_MS);
		return () => clearInterval(reading);
	});

	function show() {
		error = null;
		reason = '';
		dialogOpen = true;
	}

	async function send(event: SubmitEvent) {
		event.preventDefault();
		const result = await askCustomer({ kind, id }, Number(minutes), reason);
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		dialogOpen = false;
		await read();
	}

	async function withdraw(request: ConnectorRequest) {
		await withdrawConnectorRequest(request.id);
		await read();
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

{#if latest?.status === 'pending'}
	<span class="chip">
		<Clock size={13} class="text-warning" aria-hidden="true" />
		<span>{m.ask_customer_pending()}</span>
		<button
			type="button"
			class="ml-1 text-xs text-ink-2 underline hover:text-ink"
			onclick={() => latest && withdraw(latest)}
		>
			{m.ask_customer_withdraw()}
		</button>
	</span>
{:else}
	{#if latest?.status === 'approved' && latest.until && new Date(latest.until) > new Date()}
		<span class="chip">
			<CircleCheck size={13} class="text-ok" aria-hidden="true" />
			<span>
				{m.ask_customer_approved({
					by: latest.answered_by ?? '',
					until: time.format(new Date(latest.until))
				})}
			</span>
		</span>
	{:else if latest?.status === 'refused'}
		<span class="chip">
			<CircleX size={13} class="text-critical" aria-hidden="true" />
			<span>{m.ask_customer_refused({ by: latest.answered_by ?? '' })}</span>
		</span>
	{:else if latest?.status === 'expired'}
		<span class="chip">
			<CircleX size={13} class="text-ink-2" aria-hidden="true" />
			<span>{m.ask_customer_expired()}</span>
		</span>
	{/if}
	<button type="button" class="chip hover:bg-surface-2" onclick={show}>
		<Send size={13} aria-hidden="true" />
		<span>{m.ask_customer()}</span>
	</button>
{/if}

<Dialog bind:open={dialogOpen} title={m.ask_customer_title()}>
	<form onsubmit={send}>
		<p class="text-sm text-ink-2">{m.ask_customer_hint()}</p>
		<label class={label} for="ask-minutes">{m.request_duration()}</label>
		<select id="ask-minutes" class={field} bind:value={minutes}>
			{#each DURATIONS as option (option)}
				<option value={option}>{durationLabel(option)}</option>
			{/each}
		</select>
		<label class={label} for="ask-reason">{m.request_reason()}</label>
		<!-- One line: the connector shows the reason as plain text. -->
		<input id="ask-reason" class={field} required maxlength="500" bind:value={reason} />
		{#if error}
			<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
		{/if}
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={() => (dialogOpen = false)}
			>
				{m.action_cancel()}
			</button>
			<button
				type="submit"
				class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
			>
				{m.ask_customer_send()}
			</button>
		</div>
	</form>
</Dialog>
