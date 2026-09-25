<script lang="ts">
	/**
	 * Settings of the instance, for administrators: the directory and who
	 * administers (#144), the mail server (#145), the certificate (#146),
	 * who states a purpose (#90),
	 * who needs a second factor (#107).
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import { addAdministrator, loadAdministrators, removeAdministrator } from '$lib/api/directory';
	import { errorMessage } from '$lib/api/errors';
	import { loadPurposePrincipals, requirePurpose, waivePurpose } from '$lib/api/journal';
	import { loadRules, requireFactor, waiveFactor } from '$lib/api/secondFactor';
	import PrincipalRules from '$lib/components/PrincipalRules.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';
	import CertificateForm from '$lib/settings/CertificateForm.svelte';
	import DirectoryForm from '$lib/settings/DirectoryForm.svelte';
	import MailForm from '$lib/settings/MailForm.svelte';
</script>

<h1 class="text-4xl font-semibold">{m.nav_settings()}</h1>

{#if !session.user?.admin}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else}
	<section
		class="mt-8 max-w-2xl rounded-card border border-line bg-surface p-6"
		aria-labelledby="directory-title"
	>
		<h2 id="directory-title" class="text-xl font-semibold">{m.directory_title()}</h2>
		<p class="mt-1 text-sm text-ink-2">{m.directory_hint()}</p>
		<DirectoryForm removable />
	</section>
	<section
		class="mt-8 max-w-2xl rounded-card border border-line bg-surface p-6"
		aria-labelledby="mail-title"
	>
		<h2 id="mail-title" class="text-xl font-semibold">{m.mail_title()}</h2>
		<p class="mt-1 text-sm text-ink-2">{m.mail_hint()}</p>
		<MailForm removable recipient={session.user.kind === 'local' ? session.user.username : ''} />
	</section>
	<section
		id="certificate"
		class="mt-8 max-w-2xl scroll-mt-4 rounded-card border border-line bg-surface p-6"
		aria-labelledby="certificate-title"
	>
		<h2 id="certificate-title" class="text-xl font-semibold">{m.certificate_title()}</h2>
		<p class="mt-1 text-sm text-ink-2">{m.certificate_hint()}</p>
		<CertificateForm />
	</section>
	<PrincipalRules
		id="administrator-search"
		title={m.administrators_title()}
		hint={m.administrators_hint()}
		load={loadAdministrators}
		add={addAdministrator}
		remove={removeAdministrator}
	/>
	<PrincipalRules
		id="purpose-search"
		title={m.purpose_rules_title()}
		hint={m.purpose_rules_hint()}
		load={loadPurposePrincipals}
		add={requirePurpose}
		remove={waivePurpose}
	/>
	<PrincipalRules
		id="factor-search"
		title={m.factor_rules_title()}
		hint={m.factor_rules_hint()}
		load={loadRules}
		add={requireFactor}
		remove={waiveFactor}
	/>
{/if}
