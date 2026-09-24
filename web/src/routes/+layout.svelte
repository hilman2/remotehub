<script lang="ts">
	import './layout.css';
	import { resolve } from '$app/paths';
	import favicon from '$lib/assets/favicon.svg';
	import LocaleSwitch from '$lib/components/LocaleSwitch.svelte';
	import ServerStatus from '$lib/components/ServerStatus.svelte';
	import ThemeSwitch from '$lib/components/ThemeSwitch.svelte';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';

	let { children } = $props();

	$effect(() => {
		document.documentElement.lang = getLocale();
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>remotehub</title>
</svelte:head>

<a
	href="#main"
	class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-20 focus:rounded-md focus:bg-surface focus:px-3 focus:py-2"
>
	{m.skip_to_content()}
</a>

<div class="flex min-h-dvh flex-col">
	<header class="sticky top-0 z-10 border-b border-line bg-page/85 backdrop-blur">
		<div class="mx-auto flex h-14 max-w-7xl items-center gap-6 px-4 sm:px-6">
			<a href={resolve('/')} class="flex items-center gap-2 font-semibold tracking-tight">
				<img src={favicon} alt="" class="size-6" />
				remotehub
			</a>
			<nav class="flex items-center gap-1 text-sm text-ink-2" aria-label={m.nav_label()}>
				<a
					href={resolve('/')}
					class="rounded-md px-2.5 py-1.5 hover:bg-surface-2 hover:text-ink"
					aria-current="page"
				>
					{m.nav_devices()}
				</a>
			</nav>
			<div class="ml-auto flex items-center gap-2">
				<LocaleSwitch />
				<ThemeSwitch />
			</div>
		</div>
	</header>

	<main id="main" class="mx-auto w-full max-w-7xl flex-1 px-4 pt-8 pb-16 sm:px-6">
		{@render children()}
	</main>

	<footer class="border-t border-line">
		<div class="mx-auto max-w-7xl px-4 py-3 sm:px-6">
			<ServerStatus />
		</div>
	</footer>
</div>
