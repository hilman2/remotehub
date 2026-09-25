<script lang="ts">
	/**
	 * Settings of the instance, for administrators: who states a purpose
	 * (#90), who needs a second factor (#107).
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import { errorMessage } from '$lib/api/errors';
	import { loadPurposePrincipals, requirePurpose, waivePurpose } from '$lib/api/journal';
	import { loadRules, requireFactor, waiveFactor } from '$lib/api/secondFactor';
	import PrincipalRules from '$lib/components/PrincipalRules.svelte';
	import { m } from '$lib/paraglide/messages';
	import { session } from '$lib/session.svelte';
</script>

<h1 class="text-4xl font-semibold">{m.nav_settings()}</h1>

{#if !session.user?.admin}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else}
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
