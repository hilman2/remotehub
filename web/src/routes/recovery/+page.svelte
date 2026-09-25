<script lang="ts">
	/**
	 * Vault recovery (#95, ADR 0009): the organisation recovery key, which
	 * personal vaults it covers, and the recoveries. Administrators ask for
	 * and carry out a recovery, security officers approve it.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Hourglass from '@lucide/svelte/icons/hourglass';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import Undo2 from '@lucide/svelte/icons/undo-2';
	import type { ApiResult } from '$lib/api/client';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { isSecurityOfficer, session } from '$lib/session.svelte';
	import CarryOut from '$lib/vault/CarryOut.svelte';
	import {
		approveRecovery,
		askRecovery,
		cancelRecovery,
		createKey,
		deleteKey,
		loadKeys,
		loadRecoveries,
		type CoveredVault,
		type Keys,
		type Recovery,
		type RecoveryKind,
		type RecoveryStatus
	} from '$lib/vault/recovery';

	/** As long as a vault's passphrase must be. */
	const MIN_PASSPHRASE = 12;

	const admin = $derived(!!session.user?.admin);
	const officer = $derived(isSecurityOfficer(session.user));

	let keys = $state<Keys | null>(null);
	let recoveries = $state<Recovery[]>([]);
	let error = $state<string | null>(null);

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });
	const at = (value: string) => time.format(new Date(value));

	const KINDS: Record<RecoveryKind, () => string> = {
		passphrase: m.recovery_kind_passphrase,
		handover: m.recovery_kind_handover
	};
	const STATUS: Record<RecoveryStatus, () => string> = {
		pending: m.recovery_status_pending,
		approved: m.recovery_status_approved,
		expired: m.recovery_status_expired,
		completed: m.recovery_status_completed
	};

	async function load() {
		const [loadedKeys, loadedRecoveries] = await Promise.all([
			admin ? loadKeys() : Promise.resolve(null),
			loadRecoveries()
		]);
		if (loadedKeys?.ok) keys = loadedKeys.data;
		else if (loadedKeys) error = errorMessage(loadedKeys.code);
		if (loadedRecoveries.ok) recoveries = loadedRecoveries.data;
		else error = errorMessage(loadedRecoveries.code);
	}

	$effect(() => {
		if (admin || officer) load();
	});

	async function act(action: Promise<ApiResult<unknown>>) {
		const result = await action;
		error = result.ok ? null : problemMessage(result);
		await load();
	}

	const current = $derived(keys?.keys[0] ?? null);
	/** Whether the vault is wrapped for the newest key, an older one, or none. */
	const coverage = (vault: CoveredVault) =>
		vault.key_id === null ? 'none' : vault.key_id === current?.id ? 'current' : 'older';

	// A new key: the passphrase for its file, then the private key once.
	let keyOpen = $state(false);
	let passphrase = $state('');
	let passphraseAgain = $state('');
	let keyError = $state<string | null>(null);
	let privateText = $state<string | null>(null);
	let busy = $state(false);

	function newKey() {
		passphrase = passphraseAgain = '';
		keyError = privateText = null;
		keyOpen = true;
	}

	async function makeKey(event: SubmitEvent) {
		event.preventDefault();
		if (passphrase.length < MIN_PASSPHRASE) {
			keyError = m.vault_passphrase_short({ count: MIN_PASSPHRASE });
			return;
		}
		if (passphrase !== passphraseAgain) {
			keyError = m.vault_passphrase_mismatch();
			return;
		}
		busy = true;
		const made = await createKey(passphrase);
		busy = false;
		if (!made.ok) {
			keyError = problemMessage(made);
			return;
		}
		passphrase = passphraseAgain = '';
		privateText = made.data.text;
		const blob = new Blob([JSON.stringify(made.data.file, null, 2)], {
			type: 'application/json'
		});
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.href = url;
		link.download = `remotehub-recovery-key-${made.data.file.key_id.slice(0, 8)}.json`;
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
		await load();
	}

	// Asking for a recovery of one vault.
	let askFor = $state<CoveredVault | null>(null);
	let askOpen = $state(false);
	let askKind = $state<RecoveryKind>('passphrase');
	let reason = $state('');

	function ask(vault: CoveredVault) {
		askFor = vault;
		askKind = 'passphrase';
		reason = '';
		error = null;
		askOpen = true;
	}

	async function sendAsk(event: SubmitEvent) {
		event.preventDefault();
		if (!askFor) return;
		const result = await askRecovery(askFor.user_id, askKind, reason);
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		askOpen = false;
		await load();
	}

	let carrying = $state<Recovery | null>(null);
	let carryOpen = $state(false);

	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
</script>

<h1 class="text-4xl font-semibold">{m.nav_recovery()}</h1>
<p class="mt-3 max-w-2xl text-sm text-ink-2">{m.recovery_intro()}</p>

{#if !admin && !officer}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else}
	{#if error}
		<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}

	{#if admin && keys}
		<section class="mt-8" aria-labelledby="recovery-keys">
			<div class="flex flex-wrap items-center gap-3">
				<h2 id="recovery-keys" class="text-lg font-semibold">{m.recovery_keys_title()}</h2>
				<button type="button" class="{button} ml-auto" onclick={newKey}>
					<KeyRound size={16} aria-hidden="true" />
					{current ? m.recovery_key_rotate() : m.recovery_key_create()}
				</button>
			</div>
			{#if keys.keys.length === 0}
				<p class="mt-2 flex items-center gap-2 text-sm text-ink-2">
					<TriangleAlert size={16} class="text-warning" aria-hidden="true" />
					{m.recovery_no_key()}
				</p>
			{:else}
				<ul class="mt-3 divide-y divide-line rounded-card border border-line bg-surface">
					{#each keys.keys as key, index (key.id)}
						<li class="flex flex-wrap items-center gap-3 px-4 py-2 text-sm">
							<span class="font-mono">{key.id.slice(0, 8)}</span>
							<span class="text-ink-2">
								{m.recovery_key_made({ name: key.created_by_name, time: at(key.created_at) })}
							</span>
							<span class="text-ink-2">{m.recovery_key_vaults({ count: key.vaults })}</span>
							{#if key.master_key}
								<span class="flex items-center gap-1 text-ink-2">
									<ShieldCheck size={14} class="text-ok" aria-hidden="true" />
									{m.recovery_key_master()}
								</span>
							{:else}
								<span class="flex items-center gap-1 text-ink-2">
									<TriangleAlert size={14} class="text-warning" aria-hidden="true" />
									{m.recovery_key_no_master()}
								</span>
							{/if}
							{#if index === 0}
								<span class="flex items-center gap-1 text-ink-2">
									<CircleCheck size={14} class="text-ok" aria-hidden="true" />
									{m.recovery_key_current()}
								</span>
							{:else if key.vaults === 0}
								<button
									type="button"
									class="ml-auto rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
									onclick={() => act(deleteKey(key.id))}
								>
									{m.catalog_delete()}
								</button>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section class="mt-8" aria-labelledby="recovery-vaults">
			<h2 id="recovery-vaults" class="text-lg font-semibold">{m.recovery_vaults_title()}</h2>
			{#if keys.vaults.length === 0}
				<p class="mt-2 text-sm text-ink-3">{m.recovery_no_vaults()}</p>
			{:else}
				<ul class="mt-3 divide-y divide-line rounded-card border border-line bg-surface">
					{#each keys.vaults as vault (vault.user_id)}
						<li class="flex flex-wrap items-center gap-3 px-4 py-2 text-sm" data-testid="vault-row">
							<span class="font-medium">{vault.display_name}</span>
							<span class="text-ink-3">{vault.username}</span>
							{#if coverage(vault) === 'current'}
								<span class="flex items-center gap-1 text-ink-2">
									<ShieldCheck size={14} class="text-ok" aria-hidden="true" />
									{m.recovery_covered()}
								</span>
							{:else if coverage(vault) === 'older'}
								<span class="flex items-center gap-1 text-ink-2">
									<Hourglass size={14} aria-hidden="true" />
									{m.recovery_covered_older()}
								</span>
							{:else}
								<span class="flex items-center gap-1 text-ink-2">
									<TriangleAlert size={14} class="text-warning" aria-hidden="true" />
									{m.recovery_not_covered()}
								</span>
							{/if}
							{#if vault.key_id}
								<button type="button" class="{button} ml-auto" onclick={() => ask(vault)}>
									{m.recovery_ask()}
								</button>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	{/if}

	<section class="mt-8" aria-labelledby="recovery-list">
		<h2 id="recovery-list" class="text-lg font-semibold">{m.recovery_list_title()}</h2>
		{#if recoveries.length === 0}
			<p class="mt-2 text-sm text-ink-3">{m.recovery_none()}</p>
		{:else}
			<ul class="mt-3 space-y-3">
				{#each recoveries as recovery (recovery.id)}
					<li class="rounded-card border border-line bg-surface p-4" data-testid="recovery">
						<p class="text-xs text-ink-3">
							{m.request_from({ name: recovery.requester_name, time: at(recovery.created_at) })}
						</p>
						<p class="font-medium">
							{m.recovery_summary({ kind: KINDS[recovery.kind](), name: recovery.user_name })}
						</p>
						<p class="mt-1 text-sm whitespace-pre-line text-ink-2">{recovery.reason}</p>
						<p class="mt-2 flex items-center gap-1.5 text-sm text-ink-2">
							{#if recovery.status === 'pending'}
								<Hourglass size={15} aria-hidden="true" />
							{:else if recovery.status === 'expired'}
								<TriangleAlert size={15} class="text-warning" aria-hidden="true" />
							{:else}
								<CircleCheck size={15} class="text-ok" aria-hidden="true" />
							{/if}
							{STATUS[recovery.status]()}
							{#if recovery.approver_name}
								· {m.recovery_approved_by({ name: recovery.approver_name })}
							{/if}
						</p>
						<div class="mt-3 flex flex-wrap gap-2">
							{#if recovery.status === 'pending' && officer && !recovery.mine}
								<button
									type="button"
									class={primary}
									onclick={() => act(approveRecovery(recovery.id))}
								>
									<CircleCheck size={16} aria-hidden="true" />
									{m.request_approve()}
								</button>
							{/if}
							{#if recovery.status === 'approved' && recovery.mine}
								<button
									type="button"
									class={primary}
									onclick={() => {
										carrying = recovery;
										carryOpen = true;
									}}
								>
									<KeyRound size={16} aria-hidden="true" />
									{m.recovery_carry_out()}
								</button>
							{/if}
							{#if recovery.status !== 'completed' && admin}
								<button
									type="button"
									class={button}
									onclick={() => act(cancelRecovery(recovery.id))}
								>
									<Undo2 size={16} aria-hidden="true" />
									{m.request_cancel()}
								</button>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
{/if}

<Dialog bind:open={keyOpen} title={current ? m.recovery_key_rotate() : m.recovery_key_create()}>
	{#if keyOpen}
		{#if privateText}
			<p class="text-sm">{m.recovery_key_shown_once()}</p>
			<p
				class="mt-4 rounded-lg bg-surface-2 p-3 font-mono text-lg break-all"
				data-testid="organisation-private-key"
			>
				{privateText}
			</p>
			<button type="button" class="{primary} mt-5" onclick={() => (keyOpen = false)}>
				{m.vault_recovery_saved()}
			</button>
		{:else}
			<form onsubmit={makeKey}>
				<p class="text-sm text-ink-2">{m.recovery_key_create_hint()}</p>
				<label class={label} for="recovery-new-passphrase">{m.recovery_key_passphrase()}</label>
				<input
					id="recovery-new-passphrase"
					class={field}
					type="password"
					required
					autocomplete="new-password"
					bind:value={passphrase}
				/>
				<label class={label} for="recovery-new-passphrase-again">
					{m.field_passphrase_again()}
				</label>
				<input
					id="recovery-new-passphrase-again"
					class={field}
					type="password"
					required
					autocomplete="new-password"
					bind:value={passphraseAgain}
				/>
				<button type="submit" class="{primary} mt-5" disabled={busy}>
					{m.recovery_key_create()}
				</button>
				{#if keyError}
					<p class="mt-3 text-sm text-critical" role="alert">{keyError}</p>
				{/if}
			</form>
		{/if}
	{/if}
</Dialog>

<Dialog bind:open={askOpen} title={m.recovery_ask()}>
	{#if askOpen && askFor}
		<form onsubmit={sendAsk}>
			<p class="text-sm font-medium">{askFor.display_name}</p>
			<fieldset class="mt-3">
				<legend class="text-sm font-medium">{m.recovery_kind()}</legend>
				{#each ['passphrase', 'handover'] as const as kind (kind)}
					<label class="mt-2 flex items-center gap-2 text-sm">
						<input type="radio" name="recovery-kind" value={kind} bind:group={askKind} />
						{KINDS[kind]()}
					</label>
				{/each}
			</fieldset>
			<label class={label} for="recovery-reason">{m.request_reason()}</label>
			<textarea
				id="recovery-reason"
				class={field}
				rows="3"
				required
				maxlength="500"
				bind:value={reason}></textarea>
			<button type="submit" class="{primary} mt-5">{m.recovery_ask()}</button>
			{#if error}
				<p class="mt-3 text-sm text-critical" role="alert">{error}</p>
			{/if}
		</form>
	{/if}
</Dialog>

<Dialog bind:open={carryOpen} title={m.recovery_carry_out()}>
	{#if carryOpen && carrying}
		<CarryOut recovery={carrying} ondone={load} />
	{/if}
</Dialog>
