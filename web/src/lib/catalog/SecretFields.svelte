<script module lang="ts">
	import type { CredentialKind } from '$lib/api/catalog';

	export interface SecretValues {
		password: string;
		privateKey: string;
		passphrase: string;
		certificate: string;
	}

	/**
	 * The secret fields to send for `kind`: all that were filled in, or none
	 * if `keep` allows keeping the stored secret and nothing new was given.
	 */
	export function secretInput(
		kind: CredentialKind,
		keep: boolean,
		values: SecretValues
	): { password?: string; private_key?: string; passphrase?: string; certificate?: string } {
		if (kind === 'password') {
			return values.password || !keep ? { password: values.password } : {};
		}
		if (!values.privateKey.trim() && keep) return {};
		return {
			private_key: values.privateKey,
			...(values.passphrase ? { passphrase: values.passphrase } : {}),
			...(values.certificate.trim() ? { certificate: values.certificate } : {})
		};
	}
</script>

<script lang="ts">
	/**
	 * The secret of a credential or of a device's own credentials: a password,
	 * or an SSH key with its passphrase and certificate, typed or loaded from
	 * a file. With `keep`, fields left empty keep what is stored.
	 */
	import { m } from '$lib/paraglide/messages';

	let {
		id,
		kind,
		keep,
		password = $bindable(''),
		privateKey = $bindable(''),
		passphrase = $bindable(''),
		certificate = $bindable('')
	}: {
		/** Prefix of the fields' IDs. */
		id: string;
		kind: CredentialKind;
		keep: boolean;
		password?: string;
		privateKey?: string;
		passphrase?: string;
		certificate?: string;
	} = $props();

	/** Keys and certificates are a few KiB; anything much larger is the wrong file. */
	const MAX_FILE_BYTES = 64 * 1024;

	async function load(event: Event, into: (text: string) => void) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (file && file.size <= MAX_FILE_BYTES) into(await file.text());
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
	const fileButton =
		'mt-3 cursor-pointer rounded-md px-2 py-0.5 text-xs text-ink-3 underline hover:text-ink focus-within:outline-2';
</script>

{#if kind === 'password'}
	<label class={label} for="{id}-password">{m.field_password()}</label>
	<input
		id="{id}-password"
		class={field}
		type="password"
		autocomplete="new-password"
		required={!keep}
		bind:value={password}
		aria-describedby={keep ? `${id}-secret-hint` : undefined}
	/>
	{#if keep}
		<p id="{id}-secret-hint" class="mt-1 text-xs text-ink-3">{m.field_password_keep()}</p>
	{/if}
{:else}
	<div class="flex items-baseline justify-between">
		<label class={label} for="{id}-private-key">{m.field_private_key()}</label>
		<label class={fileButton}>
			{m.field_load_private_key()}
			<input
				type="file"
				class="sr-only"
				onchange={(event) => load(event, (text) => (privateKey = text))}
			/>
		</label>
	</div>
	<textarea
		id="{id}-private-key"
		class="{field} h-28 font-mono text-xs"
		spellcheck="false"
		required={!keep}
		bind:value={privateKey}
		aria-describedby={keep ? `${id}-secret-hint` : undefined}></textarea>
	{#if keep}
		<p id="{id}-secret-hint" class="mt-1 text-xs text-ink-3">{m.field_key_keep()}</p>
	{/if}

	<label class={label} for="{id}-passphrase">{m.field_passphrase()}</label>
	<input
		id="{id}-passphrase"
		class={field}
		type="password"
		autocomplete="off"
		bind:value={passphrase}
	/>

	<div class="flex items-baseline justify-between">
		<label class={label} for="{id}-certificate">{m.field_certificate()}</label>
		<label class={fileButton}>
			{m.field_load_certificate()}
			<input
				type="file"
				class="sr-only"
				onchange={(event) => load(event, (text) => (certificate = text))}
			/>
		</label>
	</div>
	<textarea
		id="{id}-certificate"
		class="{field} h-16 font-mono text-xs"
		spellcheck="false"
		bind:value={certificate}></textarea>
{/if}
