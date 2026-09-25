<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import LogIn from '@lucide/svelte/icons/log-in';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { PROTOCOLS } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import ProtocolChip from '$lib/catalog/ProtocolChip.svelte';
	import AuthenticatorSetup from '$lib/components/AuthenticatorSetup.svelte';
	import Logo from '$lib/components/Logo.svelte';
	import { getCredential, webauthnAvailable } from '$lib/kratos/webauthn';
	import {
		endSession,
		loadFlow,
		messages,
		offers,
		startFlow,
		value,
		submitFlow,
		type Flow,
		type FlowResult,
		type UiText
	} from '$lib/kratos/flow';
	import Messages from '$lib/kratos/Messages.svelte';
	import { m } from '$lib/paraglide/messages';
	import { loadMethods, signIn, signInLocal, type Methods } from '$lib/session.svelte';

	/** Kratos' message for a wrong identifier or password. */
	const KRATOS_INVALID_CREDENTIALS = 4000006;

	let methods = $state<Methods>({ directory: true, local: false, providers: [] });
	let username = $state('');
	let password = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	// Local accounts: password first, then the second factor.
	let second = $state<Flow | null>(null);
	let code = $state('');
	let useRecovery = $state(false);
	let texts = $state<UiText[]>([]);
	// Directory accounts (#107): a code of their app, or setting one up with
	// the key the server offers. The password is sent again with it.
	let directoryFactor = $state<'code' | { secret: string; uri: string } | null>(null);

	$effect(() => {
		// Read once: navigating on from here must not run this again.
		const query = new URLSearchParams(location.search);
		loadMethods().then(async (result) => {
			if (!result.ok) return;
			methods = result.data;
			// Recovery of an account with a second factor ends here.
			if (methods.local && page.state.secondFactor) askSecond();
			// Back from a provider (#109): signed in to Kratos; or Kratos asks
			// for the second factor in a flow of its own; or it refused, with
			// the flow's messages.
			if (query.get('from') === 'provider') {
				busy = true;
				await establish();
				busy = false;
			} else if (query.get('flow')) {
				const back = await loadFlow('login', query.get('flow')!);
				if (back.kind !== 'flow') return;
				if (back.flow.requested_aal === 'aal2') second = back.flow;
				else texts = messages(back.flow);
			}
		});
	});

	/**
	 * Signs in with a passkey instead of the password (#112). The account's
	 * second factor follows as after the password.
	 */
	async function signInWithPasskey() {
		busy = true;
		error = null;
		texts = [];
		await endSession();
		const started = await startFlow('login');
		if (started.kind !== 'flow') {
			await follow(started);
			return;
		}
		let passkey: string;
		try {
			passkey = await getCredential(String(value(started.flow, 'passkey_challenge')));
		} catch {
			busy = false;
			error = m.passkey_failed();
			return;
		}
		await follow(await submitFlow(started.flow, { method: 'passkey', passkey_login: passkey }));
	}

	/** The second factor with a security key or passkey instead of the app's code. */
	async function secondWithKey() {
		if (!second) return;
		busy = true;
		error = null;
		let key: string;
		try {
			key = await getCredential(String(value(second, 'webauthn_login_trigger')));
		} catch {
			busy = false;
			error = m.passkey_failed();
			return;
		}
		// Kratos wants the identifier back, which the flow names itself.
		const identifier = value(second, 'identifier');
		await follow(await submitFlow(second, { method: 'webauthn', webauthn_login: key, identifier }));
	}

	/**
	 * Signs in through an OpenID Connect provider (#109). The browser leaves
	 * for the provider and comes back through Kratos to this page, with
	 * `from=provider` once Kratos signed the account in.
	 */
	async function signInWith(provider: string) {
		busy = true;
		error = null;
		texts = [];
		await endSession();
		const back = new URL(resolve('/sign-in'), location.href);
		back.searchParams.set('from', 'provider');
		const started = await startFlow('login', `?return_to=${encodeURIComponent(back.href)}`);
		if (started.kind !== 'flow') {
			await follow(started);
			return;
		}
		const result = await submitFlow(started.flow, { method: 'oidc', provider });
		if (result.kind === 'redirect') {
			location.assign(result.to.href);
			return;
		}
		await follow(result);
	}

	/**
	 * One form for both kinds of account (#121). A name without `@` is a
	 * directory account. An address is tried as a local account first; if
	 * Kratos does not take it, the directory gets it, where it may be a
	 * user principal name. Nothing tells beforehand which kind an address
	 * is: that would let anyone find out which local accounts exist.
	 */
	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		texts = [];
		const address = username.includes('@');
		if (!methods.local || (!address && methods.directory)) {
			await signInDirectory();
			return;
		}
		// Typed credentials count, not a session someone left in this browser.
		await endSession();
		const started = await startFlow('login');
		if (started.kind !== 'flow') {
			await follow(started);
			return;
		}
		const result = await submitFlow(started.flow, {
			method: 'password',
			identifier: username.trim(),
			password
		});
		const refused =
			result.kind === 'flow' &&
			messages(result.flow).some((text) => text.id === KRATOS_INVALID_CREDENTIALS);
		if (refused && methods.directory) {
			await signInDirectory();
			return;
		}
		password = '';
		await follow(result);
	}

	/** Signs in with the directory, with a code of the app if there is one. */
	async function signInDirectory(factor: { code?: string; totp_secret?: string } = {}) {
		busy = true;
		error = null;
		const result = await signIn(username, password, factor);
		busy = false;
		if (result.ok) {
			password = '';
			await goto(resolve('/'));
		} else if (result.code === 'second_factor_required') {
			directoryFactor = 'code';
		} else if (result.code === 'second_factor_setup_required') {
			directoryFactor = { secret: String(result.params.secret), uri: String(result.params.uri) };
		} else {
			error = errorMessage(result.code);
			// A wrong code may be typed again; anything else starts over.
			if (result.code !== 'second_factor_invalid') {
				password = '';
				directoryFactor = null;
			}
		}
	}

	async function submitDirectoryCode(event: SubmitEvent) {
		event.preventDefault();
		const typed = code.trim();
		code = '';
		await signInDirectory({ code: typed });
	}

	/** Goes on from what Kratos answered. */
	async function follow(result: FlowResult) {
		busy = false;
		switch (result.kind) {
			case 'done':
				return establish();
			case 'redirect':
				return result.to.searchParams.get('aal') === 'aal2' ? askSecond() : establish();
			case 'flow':
				texts = messages(result.flow);
				return;
			case 'expired':
				error = m.kratos_expired();
				return;
			case 'failed':
				error = errorMessage('accounts_unavailable');
		}
	}

	async function askSecond() {
		const started = await startFlow('login', '?aal=aal2');
		if (started.kind === 'flow') {
			second = started.flow;
			texts = [];
		} else {
			await follow(started);
		}
	}

	async function submitSecond(event: SubmitEvent) {
		event.preventDefault();
		if (!second) return;
		busy = true;
		const values = useRecovery
			? { method: 'lookup_secret', lookup_secret: code.trim() }
			: { method: 'totp', totp_code: code.trim() };
		code = '';
		await follow(await submitFlow(second, values));
	}

	/** remotehub's own session, for the account Kratos signed in. */
	async function establish() {
		const result = await signInLocal();
		if (result.ok) {
			await goto(resolve('/'));
		} else if (result.code === 'second_factor_required') {
			await askSecond();
		} else if (result.code === 'second_factor_setup_required') {
			await goto(resolve('/sign-in/setup'));
		} else {
			error = errorMessage(result.code);
		}
	}

	const field = 'h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5';
</script>

<div class="grid min-h-dvh lg:grid-cols-2">
	<section
		class="relative hidden flex-col overflow-hidden border-r border-line bg-sunken px-16 py-14 lg:flex"
		style="background-image: linear-gradient(var(--line) 1px, transparent 1px), linear-gradient(90deg, var(--line) 1px, transparent 1px); background-size: 40px 40px"
	>
		<div class="flex items-center gap-3">
			<Logo size={32} />
			<span class="font-display text-xl font-bold">remotehub</span>
		</div>
		<p
			class="mt-auto mb-auto font-display text-6xl leading-[0.98] font-semibold tracking-tight whitespace-pre-line xl:text-7xl"
		>
			{m.sign_in_headline()}
		</p>
		<div class="flex gap-2.5">
			{#each PROTOCOLS as protocol (protocol)}
				<ProtocolChip {protocol} large />
			{/each}
		</div>
	</section>

	<section class="flex items-center justify-center px-6 py-16">
		<div class="flex w-full max-w-sm flex-col gap-5">
			<div class="mb-2 flex items-center gap-3 lg:hidden">
				<Logo size={28} />
				<span class="font-display text-lg font-bold">remotehub</span>
			</div>
			<h1 class="text-3xl font-semibold">{m.sign_in_title()}</h1>

			{#if second}
				<form class="flex flex-col gap-5" onsubmit={submitSecond}>
					<p class="text-sm text-ink-2">
						{useRecovery ? m.sign_in_recovery_hint() : m.sign_in_second_hint()}
					</p>
					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="second-code">
							{useRecovery ? m.sign_in_recovery_code() : m.sign_in_code()}
						</label>
						<input
							id="second-code"
							class="{field} font-mono tracking-widest"
							autocomplete="one-time-code"
							inputmode={useRecovery ? 'text' : 'numeric'}
							required
							bind:value={code}
						/>
					</div>
					<Messages {texts} />
					{#if error}
						<p class="flex items-start gap-2 text-sm" role="alert">
							<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
							{error}
						</p>
					{/if}
					<button
						type="submit"
						disabled={busy}
						class="inline-flex h-13 w-full items-center justify-center gap-2 rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
					>
						<LogIn size={18} aria-hidden="true" />
						{busy ? m.sign_in_busy() : m.sign_in_confirm()}
					</button>
					{#if offers(second, 'webauthn_login_trigger') && webauthnAvailable()}
						<button
							type="button"
							disabled={busy}
							class="inline-flex h-12 w-full items-center justify-center gap-2 rounded-xl border border-line-strong bg-surface font-medium hover:bg-surface-2 disabled:opacity-60"
							onclick={secondWithKey}
						>
							<Fingerprint size={18} aria-hidden="true" />
							{m.sign_in_use_key()}
						</button>
					{/if}
					<button
						type="button"
						class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
						onclick={() => (useRecovery = !useRecovery)}
					>
						{useRecovery ? m.sign_in_use_totp() : m.sign_in_use_recovery()}
					</button>
				</form>
			{:else if directoryFactor === 'code'}
				<form class="flex flex-col gap-5" onsubmit={submitDirectoryCode}>
					<p class="text-sm text-ink-2">{m.sign_in_second_hint()}</p>
					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="directory-code">{m.sign_in_code()}</label>
						<input
							id="directory-code"
							class="{field} font-mono tracking-widest"
							autocomplete="one-time-code"
							inputmode="numeric"
							required
							bind:value={code}
						/>
					</div>
					{#if error}
						<p class="flex items-start gap-2 text-sm" role="alert">
							<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
							{error}
						</p>
					{/if}
					<button
						type="submit"
						disabled={busy}
						class="inline-flex h-13 w-full items-center justify-center gap-2 rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
					>
						<LogIn size={18} aria-hidden="true" />
						{busy ? m.sign_in_busy() : m.sign_in_confirm()}
					</button>
				</form>
			{:else if directoryFactor}
				{@const offer = directoryFactor}
				<p class="text-sm font-medium">{m.factor_setup_required()}</p>
				<AuthenticatorSetup
					secret={offer.secret}
					uri={offer.uri}
					{busy}
					onconfirm={(typed) => signInDirectory({ code: typed, totp_secret: offer.secret })}
				/>
				{#if error}
					<p class="flex items-start gap-2 text-sm" role="alert">
						<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
						{error}
					</p>
				{/if}
			{:else}
				<form class="flex flex-col gap-5" onsubmit={submit}>
					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="username">
							{!methods.directory
								? m.sign_in_email()
								: methods.local
									? m.sign_in_username_or_email()
									: m.sign_in_username()}
						</label>
						<input
							id="username"
							name="username"
							type="text"
							autocomplete="username"
							required
							bind:value={username}
							class={field}
						/>
					</div>

					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="password">{m.sign_in_password()}</label>
						<input
							id="password"
							name="password"
							type="password"
							autocomplete="current-password"
							required
							bind:value={password}
							class={field}
						/>
					</div>

					<Messages {texts} />
					{#if error}
						<p class="flex items-start gap-2 text-sm" role="alert">
							<CircleAlert size={16} class="mt-0.5 shrink-0 text-critical" aria-hidden="true" />
							{error}
						</p>
					{/if}

					<button
						type="submit"
						disabled={busy}
						class="inline-flex h-13 w-full items-center justify-center gap-2 rounded-xl bg-accent font-display text-lg font-semibold text-accent-ink hover:brightness-110 disabled:opacity-60"
					>
						<LogIn size={18} aria-hidden="true" />
						{busy ? m.sign_in_busy() : m.sign_in_submit()}
					</button>
					{#if methods.local}
						<a
							href={resolve('/sign-in/recovery')}
							class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
						>
							{m.sign_in_forgot()}
						</a>
					{/if}
				</form>
				{#if methods.local && (methods.providers.length > 0 || webauthnAvailable())}
					<div class="flex items-center gap-3 text-sm text-ink-3">
						<span class="h-px flex-1 bg-line"></span>
						{m.sign_in_or()}
						<span class="h-px flex-1 bg-line"></span>
					</div>
					<div class="flex flex-col gap-2">
						{#if webauthnAvailable()}
							<button
								type="button"
								disabled={busy}
								class="inline-flex h-12 w-full items-center justify-center gap-2 rounded-xl border border-line-strong bg-surface font-medium hover:bg-surface-2 disabled:opacity-60"
								onclick={signInWithPasskey}
							>
								<Fingerprint size={18} aria-hidden="true" />
								{m.sign_in_passkey()}
							</button>
						{/if}
						{#each methods.providers as provider (provider.id)}
							<button
								type="button"
								disabled={busy}
								class="inline-flex h-12 w-full items-center justify-center gap-2 rounded-xl border border-line-strong bg-surface font-medium hover:bg-surface-2 disabled:opacity-60"
								onclick={() => signInWith(provider.id)}
							>
								<KeyRound size={18} aria-hidden="true" />
								{m.sign_in_with({ provider: provider.label })}
							</button>
						{/each}
					</div>
				{/if}
			{/if}

			<a
				href={resolve('/sign-in/break-glass')}
				class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
			>
				{m.sign_in_break_glass_link()}
			</a>
		</div>
	</section>
</div>
