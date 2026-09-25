<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import LogIn from '@lucide/svelte/icons/log-in';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { PROTOCOLS } from '$lib/api/catalog';
	import { errorMessage } from '$lib/api/errors';
	import ProtocolChip from '$lib/catalog/ProtocolChip.svelte';
	import Logo from '$lib/components/Logo.svelte';
	import {
		endSession,
		messages,
		startFlow,
		submitFlow,
		type Flow,
		type FlowResult,
		type UiText
	} from '$lib/kratos/flow';
	import Messages from '$lib/kratos/Messages.svelte';
	import { m } from '$lib/paraglide/messages';
	import { loadMethods, signIn, signInLocal, type Methods } from '$lib/session.svelte';

	type Way = 'directory' | 'local';
	/** The way chosen last, in this browser only. */
	const WAY_KEY = 'remotehub.sign-in';

	let methods = $state<Methods>({ directory: true, local: false });
	let way = $state<Way>('directory');
	let username = $state('');
	let password = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	// Local accounts: password first, then the second factor.
	let second = $state<Flow | null>(null);
	let code = $state('');
	let useRecovery = $state(false);
	let texts = $state<UiText[]>([]);

	$effect(() => {
		loadMethods().then((result) => {
			if (!result.ok) return;
			methods = result.data;
			let remembered: string | null = null;
			try {
				remembered = localStorage.getItem(WAY_KEY);
			} catch {
				// No storage: the default stays.
			}
			if (!methods.directory || (methods.local && remembered === 'local')) way = 'local';
			// Recovery of an account with a second factor ends here.
			if (methods.local && page.state.secondFactor) {
				way = 'local';
				askSecond();
			}
		});
	});

	function choose(next: Way) {
		way = next;
		error = null;
		texts = [];
		second = null;
		try {
			localStorage.setItem(WAY_KEY, next);
		} catch {
			// Only a convenience.
		}
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		texts = [];
		if (way === 'directory') {
			const result = await signIn(username, password);
			password = '';
			busy = false;
			if (result.ok) await goto(resolve('/'));
			else error = errorMessage(result.code);
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
		password = '';
		await follow(result);
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
	const tab =
		'flex-1 rounded-lg px-3 py-2 text-sm font-medium text-ink-2 hover:text-ink aria-pressed:bg-surface aria-pressed:text-ink aria-pressed:shadow-sm';
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

			{#if methods.directory && methods.local && !second}
				<div class="flex gap-1 rounded-xl bg-sunken p-1" role="group" aria-label={m.sign_in_way()}>
					<button
						type="button"
						class={tab}
						aria-pressed={way === 'local'}
						onclick={() => choose('local')}
					>
						{m.sign_in_way_local()}
					</button>
					<button
						type="button"
						class={tab}
						aria-pressed={way === 'directory'}
						onclick={() => choose('directory')}
					>
						{m.sign_in_way_directory()}
					</button>
				</div>
			{/if}

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
					<button
						type="button"
						class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
						onclick={() => (useRecovery = !useRecovery)}
					>
						{useRecovery ? m.sign_in_use_totp() : m.sign_in_use_recovery()}
					</button>
				</form>
			{:else}
				<form class="flex flex-col gap-5" onsubmit={submit}>
					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="username">
							{way === 'local' ? m.sign_in_email() : m.sign_in_username()}
						</label>
						<input
							id="username"
							name="username"
							type={way === 'local' ? 'email' : 'text'}
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
					{#if way === 'local'}
						<a
							href={resolve('/sign-in/recovery')}
							class="self-center text-sm text-ink-3 hover:text-ink hover:underline"
						>
							{m.sign_in_forgot()}
						</a>
					{/if}
				</form>
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
