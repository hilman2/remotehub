<script lang="ts">
	/**
	 * The setup wizard of a fresh installation (#143). The link the installer
	 * prints carries a one-time code in its fragment; with it, the first step
	 * creates the administrator, a local account, and hands over to its
	 * invitation. Signed in, the administrator creates the break-glass
	 * account and the organisation recovery key, and finishes.
	 */
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Circle from '@lucide/svelte/icons/circle';
	import CircleDot from '@lucide/svelte/icons/circle-dot';
	import Printer from '@lucide/svelte/icons/printer';
	import { afterNavigate, goto, replaceState } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import {
		STEP,
		checkCode,
		completeSetup,
		createAdministrator,
		createBreakGlass,
		loadSetupStatus,
		saveStep,
		type BreakGlassAccount,
		type SetupStatus
	} from '$lib/api/setup';
	import Logo from '$lib/components/Logo.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session, setup } from '$lib/session.svelte';
	import BreakGlassSheet from '$lib/setup/BreakGlassSheet.svelte';
	import NewRecoveryKey from '$lib/vault/NewRecoveryKey.svelte';

	let status = $state<SetupStatus | null>(null);
	let error = $state<string | null>(null);
	let busy = $state(false);

	/** The setup code from the link; gone from the address bar at once. */
	let code = $state<string | null>(null);
	/** Whether the server took the code: unknown until it answered. */
	let codeValid = $state<boolean | null>(null);

	// replaceState throws until the router is ready, which afterNavigate
	// tells on a fresh load. A link opened on this page already changes only
	// the fragment, which afterNavigate does not see: the effect does.
	let ready = $state(false);
	afterNavigate(() => {
		ready = true;
	});
	$effect(() => {
		if (!ready) return;
		const fromLink = new URLSearchParams(page.url.hash.slice(1)).get('code');
		if (fromLink) {
			code = fromLink;
			replaceState(resolve('/setup'), {});
		}
		load();
	});

	async function load() {
		const loaded = await loadSetupStatus();
		if (!loaded.ok) {
			error = errorMessage(loaded.code);
			return;
		}
		status = loaded.data;
		setup.phase = status.phase;
		if (status.phase === 'complete') {
			await goto(resolve('/'));
		} else if (status.phase === 'pending' && code) {
			const checked = await checkCode(code);
			codeValid = checked.ok;
			// A stale link has its own text below; anything else is an error.
			if (!checked.ok && checked.code !== 'setup_code_invalid') error = problemMessage(checked);
		}
	}

	// Step 1: the administrator.
	let email = $state('');
	let name = $state('');

	async function createAdmin(event: SubmitEvent) {
		event.preventDefault();
		if (!code) return;
		busy = true;
		error = null;
		const created = await createAdministrator(code, email.trim(), name.trim());
		busy = false;
		if (!created.ok) {
			error = problemMessage(created);
			return;
		}
		setup.phase = 'administrator';
		// The invitation's page takes its flow and code from here.
		await goto(resolve('/sign-in/recovery'), {
			state: {
				recoveryFlow: new URL(created.data.link).searchParams.get('flow') ?? undefined,
				recoveryCode: created.data.code
			}
		});
	}

	/** The steps the administrator goes through, in order. */
	const STEPS = [
		{ step: STEP.administrator, label: m.wizard_step_administrator },
		{ step: STEP.breakGlass, label: m.wizard_step_break_glass },
		{ step: STEP.recoveryKey, label: m.wizard_step_recovery_key },
		{ step: STEP.done, label: m.wizard_step_done }
	];
	/** The step to do now: the first one after the last one done. */
	const current = $derived(
		status ? (STEPS.find((s) => s.step > status!.step)?.step ?? STEP.done) : STEP.administrator
	);
	const settingUp = $derived(status?.phase === 'administrator' && !!session.user?.admin);

	/** A step is done or skipped: the wizard goes on after it. */
	async function next(step: number) {
		busy = true;
		const saved = await saveStep(step);
		busy = false;
		if (!saved.ok) {
			error = problemMessage(saved);
			return;
		}
		error = null;
		skipping = false;
		stored = false;
		if (status) status.step = step;
	}

	// Skipping takes a second click.
	let skipping = $state(false);
	// "Printed and stored" before going on.
	let stored = $state(false);

	// Step 4: the break-glass account.
	let breakGlass = $state<BreakGlassAccount | null>(null);
	let breakGlassExists = $state(false);

	async function makeBreakGlass() {
		busy = true;
		error = null;
		const made = await createBreakGlass();
		busy = false;
		if (made.ok) breakGlass = made.data;
		else if (made.code === 'break_glass_exists') breakGlassExists = true;
		else error = problemMessage(made);
	}

	// Step 5: the recovery key, made by NewRecoveryKey.
	let keyMade = $state(false);

	async function finish() {
		busy = true;
		const done = await completeSetup();
		busy = false;
		if (!done.ok) {
			error = problemMessage(done);
			return;
		}
		setup.phase = 'complete';
		await goto(resolve('/'));
	}

	const primary =
		'inline-flex items-center gap-1.5 rounded-xl bg-accent px-4 py-2 text-sm font-semibold text-accent-ink disabled:opacity-60';
	const button =
		'inline-flex items-center gap-1.5 rounded-xl border border-line bg-surface px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-60';
	const field = 'h-12 w-full rounded-xl border border-line-strong bg-surface px-3.5';
</script>

{#snippet skip(step: number)}
	{#if skipping}
		<p class="text-sm text-warning" role="alert">{m.wizard_skip_warning()}</p>
		<div class="flex gap-2">
			<button type="button" class={button} disabled={busy} onclick={() => next(step)}>
				{m.wizard_skip_confirm()}
			</button>
			<button type="button" class={button} onclick={() => (skipping = false)}>
				{m.action_cancel()}
			</button>
		</div>
	{:else}
		<button
			type="button"
			class="self-start text-sm text-ink-2 hover:underline"
			onclick={() => (skipping = true)}
		>
			{m.wizard_skip()}
		</button>
	{/if}
{/snippet}

{#snippet storedAndOn(step: number)}
	<label class="flex items-center gap-2 text-sm print:hidden">
		<input type="checkbox" bind:checked={stored} />
		{m.wizard_stored()}
	</label>
	<div class="flex flex-wrap gap-2 print:hidden">
		<button type="button" class={button} onclick={() => window.print()}>
			<Printer size={14} aria-hidden="true" />
			{m.print()}
		</button>
		<button type="button" class={primary} disabled={!stored || busy} onclick={() => next(step)}>
			{m.setup_continue()}
		</button>
	</div>
{/snippet}

<section class="flex min-h-dvh justify-center px-6 py-16">
	<div class="flex w-full max-w-xl flex-col gap-6">
		<div class="flex items-center gap-3 print:hidden">
			<Logo size={28} />
			<span class="font-display text-lg font-bold">remotehub</span>
		</div>
		<h1 class="text-3xl font-semibold print:hidden">{m.wizard_title()}</h1>

		{#if status}
			<ol class="flex flex-wrap gap-x-5 gap-y-2 text-sm print:hidden" aria-label={m.wizard_steps()}>
				{#each STEPS as { step, label } (step)}
					{@const done = step <= status.step}
					<li
						class="flex items-center gap-1.5 {step === current ? 'font-semibold' : 'text-ink-2'}"
						aria-current={step === current ? 'step' : undefined}
					>
						{#if done}
							<CircleCheck size={15} class="text-ok" aria-hidden="true" />
						{:else if step === current}
							<CircleDot size={15} aria-hidden="true" />
						{:else}
							<Circle size={15} aria-hidden="true" />
						{/if}
						{label()}
						{#if done}<span class="sr-only">{m.wizard_step_is_done()}</span>{/if}
					</li>
				{/each}
			</ol>
		{/if}

		{#if error}
			<p class="text-sm text-critical" role="alert">{error}</p>
		{/if}

		{#if status?.phase === 'pending'}
			{#if !code}
				<p class="text-sm text-ink-2">{m.wizard_waiting()}</p>
				<pre class="rounded-lg bg-surface-2 p-3 font-mono text-sm">{m.wizard_command()}</pre>
			{:else if codeValid === false}
				<p class="text-sm text-ink-2">{m.wizard_link_stale()}</p>
				<pre class="rounded-lg bg-surface-2 p-3 font-mono text-sm">{m.wizard_command()}</pre>
			{:else if codeValid}
				<form class="flex flex-col gap-5" onsubmit={createAdmin}>
					<p class="text-sm text-ink-2">{m.wizard_administrator_hint()}</p>
					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="setup-name">{m.field_name()}</label>
						<input id="setup-name" class={field} autocomplete="name" bind:value={name} />
					</div>
					<div class="flex flex-col gap-2">
						<label class="text-sm font-medium" for="setup-email">{m.sign_in_email()}</label>
						<input
							id="setup-email"
							class={field}
							type="email"
							autocomplete="email"
							required
							bind:value={email}
						/>
					</div>
					<button type="submit" class="{primary} self-start" disabled={busy}>
						{m.wizard_administrator_create()}
					</button>
				</form>
			{/if}
		{:else if status?.phase === 'administrator' && !session.user}
			<p class="text-sm text-ink-2">{m.wizard_sign_in()}</p>
			<a class="{primary} self-start" href={resolve('/sign-in')}>{m.sign_in_submit()}</a>
		{:else if status?.phase === 'administrator' && !settingUp}
			<p class="text-sm text-ink-2">{m.wizard_not_yours()}</p>
		{:else if settingUp && current === STEP.breakGlass}
			<h2 class="text-xl font-semibold print:hidden">{m.wizard_step_break_glass()}</h2>
			<p class="text-sm text-ink-2 print:hidden">{m.wizard_break_glass_hint()}</p>
			{#if breakGlass}
				<BreakGlassSheet account={breakGlass} />
				{@render storedAndOn(STEP.breakGlass)}
			{:else if breakGlassExists}
				<p class="text-sm">{m.wizard_break_glass_exists()}</p>
				<button
					type="button"
					class="{primary} self-start"
					disabled={busy}
					onclick={() => next(STEP.breakGlass)}
				>
					{m.setup_continue()}
				</button>
			{:else}
				<button type="button" class="{primary} self-start" disabled={busy} onclick={makeBreakGlass}>
					{m.wizard_break_glass_create()}
				</button>
				{@render skip(STEP.breakGlass)}
			{/if}
		{:else if settingUp && current === STEP.recoveryKey}
			<h2 class="text-xl font-semibold print:hidden">{m.wizard_step_recovery_key()}</h2>
			<p class="text-sm text-ink-2 print:hidden">{m.wizard_recovery_key_hint()}</p>
			<p class="text-sm text-ink-2 print:hidden">{m.wizard_recovery_key_officer()}</p>
			<div class="rounded-card border border-line bg-surface p-6">
				<NewRecoveryKey oncreated={() => (keyMade = true)} ondone={() => next(STEP.recoveryKey)} />
			</div>
			{#if !keyMade}
				{@render skip(STEP.recoveryKey)}
			{/if}
		{:else if settingUp}
			<h2 class="text-xl font-semibold">{m.wizard_step_done()}</h2>
			<p class="text-sm text-ink-2">{m.wizard_done_hint()}</p>
			<button type="button" class="{primary} self-start" disabled={busy} onclick={finish}>
				{m.wizard_finish()}
			</button>
		{/if}
	</div>
</section>
