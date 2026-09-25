<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import ShieldAlert from '@lucide/svelte/icons/shield-alert';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import {
		auditActionLabel,
		loadAudit,
		verifyAudit,
		type AuditRecord,
		type Verification
	} from '$lib/api/audit';
	import { errorMessage } from '$lib/api/errors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { isAuditor, session } from '$lib/session.svelte';

	let records = $state<AuditRecord[]>([]);
	let error = $state<string | null>(null);
	let more = $state(false);
	let verification = $state<Verification | null>(null);
	let verifying = $state(false);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'medium' });

	async function load(before?: number) {
		const result = await loadAudit(before);
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		records = before ? [...records, ...result.data] : result.data;
		more = result.data.length === 100;
	}

	async function verify() {
		verifying = true;
		const result = await verifyAudit();
		verifying = false;
		if (result.ok) {
			verification = result.data;
			await load();
		} else {
			error = errorMessage(result.code);
		}
	}

	/** Why a sign-in failed, from the problem code in the details. */
	function reason(record: AuditRecord): string | null {
		const code = record.details.reason;
		return typeof code === 'string' ? errorMessage(code) : null;
	}

	$effect(() => {
		if (isAuditor(session.user)) load();
	});
</script>

<div class="flex flex-wrap items-start justify-between gap-4">
	<div>
		<h1 class="text-4xl font-semibold">{m.audit_title()}</h1>
	</div>
	{#if isAuditor(session.user)}
		<button
			type="button"
			disabled={verifying}
			class="inline-flex items-center gap-2 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-60"
			onclick={verify}
		>
			<ShieldCheck size={16} aria-hidden="true" />
			{m.audit_verify()}
		</button>
	{/if}
</div>

{#if verification}
	<p class="mt-4 flex items-center gap-2 text-sm" role="status">
		{#if verification.first_broken === null}
			<ShieldCheck size={16} class="text-ok" aria-hidden="true" />
			{m.audit_verify_ok({ entries: verification.entries })}
		{:else}
			<ShieldAlert size={16} class="text-critical" aria-hidden="true" />
			{m.audit_verify_broken({ seq: verification.first_broken })}
		{/if}
	</p>
{/if}

{#if !isAuditor(session.user)}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else if error}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{:else if records.length === 0}
	<p class="mt-8 text-sm text-ink-2">{m.audit_empty()}</p>
{:else}
	<div class="mt-6 overflow-x-auto rounded-card border border-line bg-surface">
		<table class="w-full text-left text-sm">
			<thead class="border-b border-line text-ink-2">
				<tr>
					<th class="px-4 py-2 font-medium">{m.audit_col_time()}</th>
					<th class="px-4 py-2 font-medium">{m.audit_col_actor()}</th>
					<th class="px-4 py-2 font-medium">{m.audit_col_action()}</th>
					<th class="px-4 py-2 font-medium">{m.audit_col_address()}</th>
				</tr>
			</thead>
			<tbody>
				{#each records as record (record.seq)}
					<tr class="border-b border-line last:border-0">
						<td class="px-4 py-2 whitespace-nowrap text-ink-2 tabular-nums">
							{time.format(new Date(record.at))}
						</td>
						<td class="px-4 py-2">{record.actor_name}</td>
						<td class="px-4 py-2">
							{auditActionLabel(record.action)}
							{#if reason(record)}
								<span class="text-ink-3"> · {reason(record)}</span>
							{/if}
						</td>
						<td class="px-4 py-2 font-mono text-xs text-ink-2">{record.address ?? ''}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
	{#if more}
		<button
			type="button"
			class="mt-4 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2"
			onclick={() => load(records.at(-1)?.seq)}
		>
			{m.audit_more()}
		</button>
	{/if}
{/if}
