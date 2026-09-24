<script lang="ts">
	import Ban from '@lucide/svelte/icons/ban';
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Hourglass from '@lucide/svelte/icons/hourglass';
	import Undo2 from '@lucide/svelte/icons/undo-2';
	import { problemMessage, errorMessage } from '$lib/api/errors';
	import type { ApiResult } from '$lib/api/client';
	import {
		approveRequest,
		cancelRequest,
		denyRequest,
		loadRequests,
		type AccessRequest,
		type RequestStatus,
		type Requests
	} from '$lib/api/requests';
	import { KIND_LABELS, ROLE_LABELS, durationLabel } from '$lib/catalog/labels';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let requests = $state<Requests | null>(null);
	let error = $state<string | null>(null);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
	const at = (value: string) => time.format(new Date(value));

	const STATUS: Record<RequestStatus, () => string> = {
		pending: m.request_status_pending,
		approved: m.request_status_approved,
		denied: m.request_status_denied,
		cancelled: m.request_status_cancelled
	};

	async function load() {
		const result = await loadRequests();
		if (result.ok) requests = result.data;
		else error = errorMessage(result.code);
	}

	async function act(action: Promise<ApiResult<unknown>>) {
		const result = await action;
		error = result.ok ? null : problemMessage(result);
		await load();
	}

	$effect(() => {
		load();
	});

	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
</script>

{#snippet what(request: AccessRequest)}
	<p class="font-medium">
		{m.request_summary({
			role: ROLE_LABELS[request.role](),
			kind: KIND_LABELS[request.object.kind](),
			name: request.object_name,
			duration: durationLabel(request.minutes)
		})}
	</p>
	<p class="mt-1 text-sm whitespace-pre-line text-ink-2">{request.reason}</p>
{/snippet}

{#snippet status(request: AccessRequest)}
	<p class="mt-2 flex items-center gap-1.5 text-sm text-ink-2">
		{#if request.status === 'pending'}
			<Hourglass size={15} aria-hidden="true" />
		{:else if request.status === 'approved'}
			<CircleCheck size={15} class="text-ok" aria-hidden="true" />
		{:else}
			<Ban size={15} aria-hidden="true" />
		{/if}
		{STATUS[request.status]()}
		{#if request.status === 'approved' && request.expires_at}
			· {m.request_until({ time: at(request.expires_at) })}
		{/if}
		{#if request.decider_name}
			· {m.request_decided_by({ name: request.decider_name })}
		{/if}
	</p>
{/snippet}

<h1 class="text-4xl font-semibold">{m.requests_title()}</h1>

{#if error}
	<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{/if}

{#if requests}
	<h2 class="mt-8 text-lg font-semibold">{m.requests_to_decide()}</h2>
	{#if requests.to_decide.length === 0}
		<p class="mt-2 text-sm text-ink-3">{m.requests_none_to_decide()}</p>
	{:else}
		<ul class="mt-3 space-y-3">
			{#each requests.to_decide as request (request.id)}
				<li class="rounded-card border border-line bg-surface p-4">
					<p class="text-xs text-ink-3">
						{m.request_from({ name: request.requester_name, time: at(request.created_at) })}
					</p>
					{@render what(request)}
					<div class="mt-3 flex flex-wrap gap-2">
						<button
							type="button"
							class="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink"
							onclick={() => act(approveRequest(request.id))}
						>
							<CircleCheck size={16} aria-hidden="true" />
							{m.request_approve()}
						</button>
						<button type="button" class={button} onclick={() => act(denyRequest(request.id))}>
							<Ban size={16} aria-hidden="true" />
							{m.request_deny()}
						</button>
					</div>
				</li>
			{/each}
		</ul>
	{/if}

	<h2 class="mt-8 text-lg font-semibold">{m.requests_mine()}</h2>
	{#if requests.mine.length === 0}
		<p class="mt-2 text-sm text-ink-3">{m.requests_none_mine()}</p>
	{:else}
		<ul class="mt-3 space-y-3">
			{#each requests.mine as request (request.id)}
				<li class="rounded-card border border-line bg-surface p-4">
					<p class="text-xs text-ink-3">{at(request.created_at)}</p>
					{@render what(request)}
					{@render status(request)}
					{#if request.status === 'pending'}
						<button
							type="button"
							class="{button} mt-3"
							onclick={() => act(cancelRequest(request.id))}
						>
							<Undo2 size={16} aria-hidden="true" />
							{m.request_cancel()}
						</button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
{/if}
