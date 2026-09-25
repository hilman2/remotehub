<script lang="ts">
	/**
	 * Security keys and passkeys of a directory user (#129), as the second
	 * factor next to the authenticator app. remotehub keeps and checks them
	 * itself; the last factor stays while a rule asks for one.
	 */
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import {
		addKey,
		loadFactor,
		offerKey,
		removeKey,
		type FactorStatus
	} from '$lib/api/secondFactor';
	import { createKey, webauthnAvailable } from '$lib/kratos/webauthn';
	import { m } from '$lib/paraglide/messages';

	let status = $state<FactorStatus | null>(null);
	let name = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);

	async function load() {
		const result = await loadFactor();
		if (result.ok) status = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		load();
	});

	async function add(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		try {
			const offer = await offerKey();
			if (!offer.ok) {
				error = problemMessage(offer);
				return;
			}
			let credential: unknown;
			try {
				credential = await createKey(JSON.stringify(offer.data.options));
			} catch {
				error = m.passkey_failed();
				return;
			}
			const added = await addKey(offer.data.challenge_id, name.trim(), credential);
			if (!added.ok) {
				error = problemMessage(added);
				return;
			}
			name = '';
			await load();
		} finally {
			busy = false;
		}
	}

	async function remove(id: string) {
		busy = true;
		const result = await removeKey(id);
		busy = false;
		error = result.ok ? null : problemMessage(result);
		await load();
	}

	const button =
		'self-start rounded-xl border border-line-strong bg-surface px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-60';
</script>

{#if status?.available && webauthnAvailable()}
	<section
		class="flex max-w-3xl flex-col gap-3 rounded-card border border-line bg-surface p-6"
		aria-labelledby="directory-keys"
	>
		<h2 id="directory-keys" class="text-lg font-semibold">{m.account_keys()}</h2>
		<p class="text-sm text-ink-2">{m.account_keys_hint()}</p>
		{#if status.keys.length > 0}
			<ul class="flex flex-col gap-2">
				{#each status.keys as key (key.id)}
					<li class="flex flex-wrap items-center gap-3 text-sm">
						<CircleCheck size={16} class="text-ok" aria-hidden="true" />
						{key.name}
						<button type="button" class={button} disabled={busy} onclick={() => remove(key.id)}>
							{m.account_key_remove({ name: key.name })}
						</button>
					</li>
				{/each}
			</ul>
		{/if}
		<form class="flex flex-wrap items-end gap-3" onsubmit={add}>
			<div class="flex flex-col gap-1">
				<label class="text-sm font-medium" for="directory-key-name">{m.account_key_name()}</label>
				<input
					id="directory-key-name"
					class="h-11 w-72 rounded-xl border border-line-strong bg-page px-3"
					required
					maxlength="100"
					bind:value={name}
				/>
			</div>
			<button type="submit" class={button} disabled={busy}>{m.account_key_add()}</button>
		</form>
		{#if error}
			<p class="text-sm text-critical" role="alert">{error}</p>
		{/if}
	</section>
{/if}
