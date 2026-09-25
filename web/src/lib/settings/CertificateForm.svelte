<script lang="ts">
	/**
	 * The certificate Caddy of the ops package serves (#146): where it comes
	 * from, the root certificate of Caddy's own CA for the clients, and a
	 * certificate of your own as PFX or PEM, or back to automatic. Behind a
	 * reverse proxy of your own there is nothing to set.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Download from '@lucide/svelte/icons/download';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import {
		loadCertificate,
		resetCertificate,
		runsOutSoon,
		uploadPem,
		uploadPfx,
		type CertificateInfo,
		type CertificateStatus,
		type Refusal,
		type Source
	} from '$lib/api/certificate';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let status = $state<CertificateStatus | null>(null);
	let format = $state<'pfx' | 'pem'>('pfx');
	let pfxFile = $state<File | null>(null);
	let password = $state('');
	let certificateFile = $state<File | null>(null);
	let keyFile = $state<File | null>(null);

	let busy = $state(false);
	let error = $state<string | null>(null);
	let done = $state<string | null>(null);
	let resetting = $state(false);

	async function load() {
		const result = await loadCertificate();
		if (result.ok) status = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		load();
	});

	/** A file's bytes in base64, as the server takes a PFX file. */
	async function base64(file: File): Promise<string> {
		const bytes = new Uint8Array(await file.arrayBuffer());
		let binary = '';
		for (const byte of bytes) binary += String.fromCharCode(byte);
		return btoa(binary);
	}

	const REFUSALS: Record<Refusal, () => string> = {
		unreadable: m.certificate_refusal_unreadable,
		no_key: m.certificate_refusal_no_key,
		pfx_password: m.certificate_refusal_pfx_password,
		key_mismatch: m.certificate_refusal_key_mismatch,
		unsupported_key: m.certificate_refusal_unsupported_key,
		wrong_name: () => m.certificate_refusal_wrong_name({ host: status?.host ?? '' }),
		not_yet_valid: m.certificate_refusal_not_yet_valid,
		expired: m.certificate_refusal_expired
	};

	function failed(problem: { code: string; params?: Record<string, unknown> }) {
		const reason = problem.params?.reason;
		error =
			problem.code === 'certificate_refused' && typeof reason === 'string' && reason in REFUSALS
				? REFUSALS[reason as Refusal]()
				: problemMessage(problem);
	}

	async function upload(event: SubmitEvent) {
		event.preventDefault();
		error = done = null;
		busy = true;
		const result =
			format === 'pfx' && pfxFile
				? await uploadPfx(await base64(pfxFile), password)
				: certificateFile && keyFile
					? await uploadPem(await certificateFile.text(), await keyFile.text())
					: null;
		busy = false;
		if (!result) return;
		if (!result.ok) {
			failed(result);
			return;
		}
		password = '';
		done = m.certificate_uploaded();
		await load();
	}

	async function reset() {
		error = done = null;
		resetting = false;
		busy = true;
		const result = await resetCertificate();
		busy = false;
		if (!result.ok) {
			failed(result);
			return;
		}
		done = m.certificate_reset_done();
		await load();
	}

	const date = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'long' });
	const until = (info: CertificateInfo) => date.format(new Date(info.not_after * 1000));
	const expired = (info: CertificateInfo) => info.not_after * 1000 < Date.now();

	function describe(source: Source, served: CertificateInfo): string {
		switch (source) {
			case 'lets_encrypt':
				return m.certificate_source_lets_encrypt();
			case 'remotehub':
				return m.certificate_source_remotehub();
			case 'own':
				return m.certificate_source_own();
			case 'other':
				return m.certificate_source_other({ issuer: served.issuer });
		}
	}

	const ready = $derived(
		format === 'pfx' ? pfxFile !== null : certificateFile !== null && keyFile !== null
	);

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2 text-sm';
	const label = 'mt-4 block text-sm font-medium';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
</script>

{#if status && !status.runs}
	<p class="mt-4 text-sm text-ink-2" data-testid="certificate-external">
		{m.certificate_external()}
	</p>
{:else if status}
	<div class="mt-4 text-sm" data-testid="certificate-served">
		{#if status.served && status.source}
			<p class="flex items-start gap-2">
				<ShieldCheck size={16} class="mt-0.5 shrink-0 text-ok" aria-hidden="true" />
				{describe(status.source, status.served)}
			</p>
			<dl class="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-ink-2">
				<dt>{m.certificate_names()}</dt>
				<dd class="font-mono break-all">{status.served.names.join(', ')}</dd>
				<dt>{m.certificate_valid_until()}</dt>
				<dd>{until(status.served)}</dd>
				<dt>{m.certificate_fingerprint()}</dt>
				<dd class="font-mono text-xs break-all">{status.served.fingerprint}</dd>
			</dl>
		{:else}
			<p class="flex items-start gap-2">
				<TriangleAlert size={16} class="mt-0.5 shrink-0 text-warning" aria-hidden="true" />
				{m.certificate_none()}
			</p>
		{/if}
	</div>

	{#if status.own}
		{@const own = status.own}
		{#if expired(own.info)}
			<p class="mt-3 flex items-start gap-2 text-sm text-critical" role="alert">
				<CircleAlert size={16} class="mt-0.5 shrink-0" aria-hidden="true" />
				{m.certificate_expired({ date: until(own.info) })}
			</p>
		{:else if runsOutSoon(own.info)}
			<p class="mt-3 flex items-start gap-2 text-sm" role="alert">
				<TriangleAlert size={16} class="mt-0.5 shrink-0 text-warning" aria-hidden="true" />
				{m.certificate_runs_out({ date: until(own.info) })}
			</p>
		{/if}
		{#if !own.chain_complete}
			<p class="mt-3 flex items-start gap-2 text-sm">
				<TriangleAlert size={16} class="mt-0.5 shrink-0 text-warning" aria-hidden="true" />
				{m.certificate_chain_incomplete()}
			</p>
		{/if}
	{/if}

	{#if status.root_fingerprint && status.source !== 'own' && status.source !== 'lets_encrypt'}
		<div class="mt-5 rounded-lg border border-line p-4">
			<h3 class="font-medium">{m.certificate_root_title()}</h3>
			<p class="mt-1 text-sm text-ink-2">{m.certificate_root_hint()}</p>
			<div class="mt-3 flex flex-wrap gap-2">
				<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a download -->
				<a class={button} href="/ca.crt" download>
					<Download size={16} aria-hidden="true" />
					{m.certificate_root_pem()}
				</a>
				<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a download -->
				<a class={button} href="/ca.cer" download>
					<Download size={16} aria-hidden="true" />
					{m.certificate_root_der()}
				</a>
			</div>
			<p class="mt-3 text-xs text-ink-3">
				{m.certificate_fingerprint()}:
				<span class="font-mono break-all">{status.root_fingerprint}</span>
			</p>
			<ul class="mt-3 list-disc space-y-1 pl-5 text-sm text-ink-2">
				<li>{m.certificate_root_windows()}</li>
				<li>{m.certificate_root_mac()}</li>
				<li>{m.certificate_root_linux()}</li>
			</ul>
		</div>
	{/if}

	<form class="mt-5" onsubmit={upload}>
		<h3 class="font-medium">{m.certificate_upload_title()}</h3>
		<p class="mt-1 text-sm text-ink-2">{m.certificate_upload_hint({ host: status.host })}</p>
		<fieldset class="mt-3">
			<legend class="sr-only">{m.certificate_format()}</legend>
			<div class="flex flex-wrap gap-4 text-sm">
				<label class="flex items-center gap-2">
					<input type="radio" name="certificate-format" value="pfx" bind:group={format} />
					{m.certificate_format_pfx()}
				</label>
				<label class="flex items-center gap-2">
					<input type="radio" name="certificate-format" value="pem" bind:group={format} />
					{m.certificate_format_pem()}
				</label>
			</div>
		</fieldset>

		{#if format === 'pfx'}
			<label class={label} for="certificate-pfx">{m.certificate_pfx_file()}</label>
			<input
				id="certificate-pfx"
				class={field}
				type="file"
				accept=".pfx,.p12"
				onchange={(event) => (pfxFile = event.currentTarget.files?.[0] ?? null)}
			/>
			<label class={label} for="certificate-password">{m.certificate_pfx_password()}</label>
			<input
				id="certificate-password"
				class={field}
				type="password"
				autocomplete="off"
				bind:value={password}
			/>
		{:else}
			<label class={label} for="certificate-chain">{m.certificate_pem_certificate()}</label>
			<input
				id="certificate-chain"
				class={field}
				type="file"
				accept=".crt,.pem,.cer"
				onchange={(event) => (certificateFile = event.currentTarget.files?.[0] ?? null)}
			/>
			<label class={label} for="certificate-key">{m.certificate_pem_key()}</label>
			<input
				id="certificate-key"
				class={field}
				type="file"
				accept=".key,.pem"
				onchange={(event) => (keyFile = event.currentTarget.files?.[0] ?? null)}
			/>
		{/if}

		<div class="mt-5 flex flex-wrap gap-2">
			<button type="submit" class={primary} disabled={busy || !ready}>
				{m.certificate_upload()}
			</button>
			{#if status.own}
				<button
					type="button"
					class="{button} ml-auto"
					disabled={busy}
					onclick={() => (resetting = true)}
				>
					{m.certificate_reset()}
				</button>
			{/if}
		</div>
	</form>

	{#if resetting}
		<div class="mt-4 rounded-lg border border-line p-3 text-sm" role="alert">
			<p>{m.certificate_reset_confirm()}</p>
			<div class="mt-3 flex gap-2">
				<button type="button" class={button} onclick={reset}>{m.certificate_reset()}</button>
				<button type="button" class={button} onclick={() => (resetting = false)}>
					{m.action_cancel()}
				</button>
			</div>
		</div>
	{/if}
{/if}

{#if error}
	<p class="mt-4 flex items-center gap-2 text-sm text-critical" role="alert">
		<CircleAlert size={16} aria-hidden="true" />
		{error}
	</p>
{/if}

{#if done}
	<p class="mt-4 flex items-center gap-2 text-sm" role="status" data-testid="certificate-done">
		<CircleCheck size={16} class="text-ok" aria-hidden="true" />
		{done}
	</p>
{/if}
