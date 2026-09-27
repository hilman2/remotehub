<script lang="ts">
	/**
	 * The login profiles of the devices area (#192): the list by folder on
	 * the left, the chosen profile with the devices that use it on the right.
	 * The page puts its tabs (`tabs`) above the list.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Lock from '@lucide/svelte/icons/lock';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import type { Snippet } from 'svelte';
	import {
		allows,
		createProfile,
		deleteProfile,
		updateProfile,
		type Profile,
		type ProfileInput,
		type Tree
	} from '$lib/api/catalog';
	import type { ApiResult } from '$lib/api/client';
	import { problemMessage } from '$lib/api/errors';
	import Dialog from '$lib/components/Dialog.svelte';
	import { formatLocale, getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { queryKey } from '$lib/search/rank';
	import { CREDENTIAL_KIND_LABELS } from './labels';
	import ProfileForm from './ProfileForm.svelte';
	import ProtocolChip from './ProtocolChip.svelte';
	import RevealSecret from './RevealSecret.svelte';
	import { pathTo } from './tree';

	let {
		tree,
		selected = $bindable(null),
		tabs,
		onchange,
		onshowdevice
	}: {
		tree: Tree;
		/** The id of the profile shown. */
		selected?: string | null;
		tabs: Snippet;
		/** After a profile was created, changed or deleted. */
		onchange: () => Promise<void>;
		onshowdevice: (id: string) => void;
	} = $props();

	let query = $state('');
	let open = $state<{ type: 'form'; profile: Profile | null } | { type: 'delete' } | null>(null);
	let dialogOpen = $state(false);
	let error = $state<string | null>(null);

	const path = (folderId: string | null) =>
		folderId
			? pathTo(tree, folderId)
					.map((f) => f.name)
					.join(' / ')
			: m.profile_top_level();
	const login = (profile: Profile) =>
		profile.domain ? `${profile.domain}\\${profile.username}` : profile.username;
	const users = (profile: Profile) => tree.devices.filter((d) => d.profile_id === profile.id);

	/** The profiles by folder, the top level first, then by path; narrowed by the search. */
	const groups = $derived.by(() => {
		const wanted = queryKey(query);
		const shown = tree.profiles.filter(
			(p) => !wanted || queryKey(`${p.name} ${login(p)} ${path(p.folder_id)}`).includes(wanted)
		);
		const byFolder: Record<string, { label: string; top: boolean; profiles: Profile[] }> = {};
		for (const profile of shown) {
			const key = profile.folder_id ?? '';
			byFolder[key] ??= {
				label: path(profile.folder_id),
				top: profile.folder_id === null,
				profiles: []
			};
			byFolder[key].profiles.push(profile);
		}
		return Object.values(byFolder).sort(
			(a, b) => Number(b.top) - Number(a.top) || a.label.localeCompare(b.label, getLocale())
		);
	});

	const profile = $derived(tree.profiles.find((p) => p.id === selected));
	/** Folders a profile may go into: those one may edit, by path. */
	const places = $derived(
		tree.folders
			.filter((f) => allows(f.role, 'edit'))
			.map((f) => ({ id: f.id, path: path(f.id) }))
			.sort((a, b) => a.path.localeCompare(b.path, getLocale()))
	);
	const mayCreate = $derived(tree.may_create_top_level || places.length > 0);

	const when = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'medium', timeStyle: 'short' });

	function show(next: NonNullable<typeof open>) {
		error = null;
		open = next;
		dialogOpen = true;
	}

	async function run<T>(change: Promise<ApiResult<T>>, after?: (data: T) => void) {
		const result = await change;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		dialogOpen = false;
		error = null;
		after?.(result.data);
		await onchange();
	}

	function save(input: ProfileInput) {
		if (open?.type !== 'form') return;
		const target = open.profile;
		run(target ? updateProfile(target.id, input) : createProfile(input), (data) => {
			const id = target?.id ?? (data as { id?: string } | undefined)?.id;
			if (id) selected = id;
		});
	}

	function remove() {
		if (!profile) return;
		run(deleteProfile(profile.id), () => (selected = null));
	}

	const button =
		'inline-flex h-10 items-center gap-2 rounded-xl border border-line-strong bg-surface px-3.5 text-sm hover:bg-surface-2';
	const cell = 'border-t border-line px-4 py-2.5';
</script>

<aside
	class="flex shrink-0 flex-col gap-4 border-b border-line bg-sunken px-3 py-5 lg:w-84 lg:overflow-y-auto lg:border-r lg:border-b-0"
>
	{@render tabs()}
	<div class="flex items-center gap-2 px-2">
		<h1 class="eyebrow">{m.profiles_title()}</h1>
		{#if mayCreate}
			<button
				type="button"
				class="ml-auto flex size-8 items-center justify-center rounded-lg border border-line-strong text-ink-2 hover:bg-surface-2 hover:text-ink"
				title={m.profile_new()}
				onclick={() => show({ type: 'form', profile: null })}
			>
				<Plus size={15} aria-hidden="true" />
				<span class="sr-only">{m.profile_new()}</span>
			</button>
		{/if}
	</div>
	<label class="relative block">
		<span class="sr-only">{m.profiles_search()}</span>
		<Search size={16} class="absolute top-3 left-3 text-ink-3" aria-hidden="true" />
		<input
			type="search"
			class="h-10 w-full rounded-xl border border-line-strong bg-page pr-3 pl-9 text-sm"
			placeholder={m.profiles_search()}
			bind:value={query}
		/>
	</label>
	{#each groups as group (group.label)}
		<section class="flex flex-col gap-0.5" aria-label={group.label}>
			<h2 class="px-2 pb-1 text-xs font-semibold text-ink-3">{group.label}</h2>
			<ul class="flex flex-col gap-0.5">
				{#each group.profiles as item (item.id)}
					<li>
						<button
							type="button"
							class="flex w-full min-w-0 items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm hover:bg-surface-2 aria-[current=true]:bg-surface-2 aria-[current=true]:ring-1 aria-[current=true]:ring-line-strong"
							aria-current={item.id === selected}
							onclick={() => (selected = item.id)}
						>
							<span class="min-w-0 flex-1">
								<span class="block truncate font-medium">{item.name}</span>
								<span class="block truncate font-mono text-xs text-ink-2">{login(item)}</span>
							</span>
							<span class="shrink-0 text-xs text-ink-3">
								{m.profile_devices({ count: users(item).length })}
							</span>
						</button>
					</li>
				{/each}
			</ul>
		</section>
	{:else}
		<p class="px-2 text-sm text-ink-2">
			{query ? m.catalog_no_match() : m.profiles_empty()}
		</p>
	{/each}
</aside>

<section
	class="flex min-w-0 flex-1 flex-col gap-7 px-5 py-8 sm:px-10 lg:overflow-y-auto lg:py-10"
	aria-live="polite"
>
	{#if error && !dialogOpen}
		<p class="flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
	{#if profile}
		<div class="flex flex-wrap items-end gap-6">
			<div class="flex min-w-0 flex-1 flex-col gap-3">
				<p class="text-sm text-ink-3">{m.profile_in({ path: path(profile.folder_id) })}</p>
				<h2 class="font-display text-4xl font-semibold break-words">{profile.name}</h2>
			</div>
			{#if allows(profile.role, 'edit')}
				<div class="flex items-center gap-2">
					<button type="button" class={button} onclick={() => show({ type: 'form', profile })}>
						<Pencil size={16} aria-hidden="true" />
						{m.catalog_edit()}
					</button>
					<button
						type="button"
						class="{button} text-critical"
						onclick={() => show({ type: 'delete' })}
					>
						<Trash2 size={16} aria-hidden="true" />
						{m.catalog_delete()}
					</button>
				</div>
			{/if}
		</div>

		<dl
			class="grid grid-cols-[minmax(8rem,14rem)_minmax(0,1fr)] rounded-card border border-line bg-surface text-sm"
		>
			<dt class="px-4 py-2.5 text-ink-2">{m.credential_kind()}</dt>
			<dd class="px-4 py-2.5">{CREDENTIAL_KIND_LABELS[profile.secret_kind]()}</dd>
			<dt class="{cell} text-ink-2">{m.field_username()}</dt>
			<dd class="{cell} font-mono">{login(profile)}</dd>
			<dt class="{cell} text-ink-2">
				{profile.secret_kind === 'ssh_key' ? m.credential_key() : m.field_password()}
			</dt>
			<dd class="{cell} flex flex-col gap-2">
				<span class="flex items-center gap-2 text-ink-2">
					<Lock size={14} class="text-ok" aria-hidden="true" />
					{m.credential_hidden()}
				</span>
				{#if profile.key_fingerprint}
					<span class="font-mono text-xs break-all text-ink-2">
						{profile.key_algorithm} · {profile.key_fingerprint}
					</span>
				{/if}
				{#if allows(profile.role, 'reveal')}
					{#key profile.id}
						<RevealSecret owner="profiles" id={profile.id} />
					{/key}
				{/if}
			</dd>
			<dt class="{cell} text-ink-2">{m.profile_changed()}</dt>
			<dd class="{cell} tabular-nums">{when.format(new Date(profile.updated_at))}</dd>
		</dl>

		<section class="flex flex-col gap-3 rounded-card border border-line bg-surface p-5">
			<h3 class="font-semibold">{m.profile_used_by({ count: users(profile).length })}</h3>
			{#if users(profile).length > 0}
				<p class="text-sm text-ink-2">{m.profile_used_by_hint()}</p>
				<ul class="flex flex-wrap gap-2">
					{#each users(profile) as device (device.id)}
						<li>
							<button
								type="button"
								class="inline-flex items-center gap-2 rounded-full border border-line px-3 py-1 text-sm hover:bg-surface-2"
								onclick={() => onshowdevice(device.id)}
							>
								<ProtocolChip protocol={device.protocol} />
								{device.name}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
		<p class="max-w-3xl text-sm text-ink-2">{m.profile_rights_hint()}</p>
	{:else}
		<p class="font-display text-2xl text-ink-3">{m.profiles_select_hint()}</p>
	{/if}
</section>

<Dialog
	bind:open={dialogOpen}
	title={open?.type === 'delete'
		? m.catalog_delete()
		: open?.type === 'form' && open.profile
			? m.catalog_edit()
			: m.profile_new()}
>
	{#if dialogOpen && open?.type === 'form'}
		<ProfileForm
			profile={open.profile}
			folderId={places[0]?.id ?? null}
			{places}
			topLevel={tree.may_create_top_level}
			onsubmit={save}
			oncancel={() => (dialogOpen = false)}
		/>
	{:else if dialogOpen && open?.type === 'delete' && profile}
		<p class="text-sm">{m.catalog_delete_confirm({ name: profile.name })}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={() => (dialogOpen = false)}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={remove}
			>
				{m.catalog_delete()}
			</button>
		</div>
	{/if}
	{#if dialogOpen && error}
		<p class="mt-3 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
</Dialog>
