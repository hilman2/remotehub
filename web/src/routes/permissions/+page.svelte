<script lang="ts">
	/**
	 * Permission reports (#110) for auditors and administrators: what a
	 * person reaches and through which grant, and who reaches a folder.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { errorMessage } from '$lib/api/errors';
	import {
		folderReport,
		loadFolders,
		loadPeople,
		userReport,
		type FolderChoice,
		type FolderReport,
		type Person,
		type UserReport
	} from '$lib/api/reports';
	import { KIND_LABELS, ROLE_LABELS } from '$lib/catalog/labels';
	import { m } from '$lib/paraglide/messages';
	import { isAuditor, session } from '$lib/session.svelte';

	let people = $state<Person[]>([]);
	let folders = $state<FolderChoice[]>([]);
	let personId = $state('');
	let folderId = $state('');
	let byPerson = $state<UserReport | null>(null);
	let byFolder = $state<FolderReport | null>(null);
	let error = $state<string | null>(null);

	const allowed = $derived(isAuditor(session.user));

	$effect(() => {
		if (!allowed) return;
		Promise.all([loadPeople(), loadFolders()]).then(([p, f]) => {
			if (p.ok) people = p.data;
			else error = errorMessage(p.code);
			if (f.ok) folders = f.data;
			else error = errorMessage(f.code);
		});
	});

	async function showPerson() {
		byPerson = null;
		if (!personId) return;
		const result = await userReport(personId);
		if (result.ok) byPerson = result.data;
		else error = errorMessage(result.code);
	}

	async function showFolder() {
		byFolder = null;
		if (!folderId) return;
		const result = await folderReport(folderId);
		if (result.ok) byFolder = result.data;
		else error = errorMessage(result.code);
	}

	const path = (names: string[]) => names.join(' / ');
	const field = 'mt-1 w-full max-w-md rounded-lg border border-line bg-page px-3 py-2';
	const cell = 'px-3 py-2 align-top';
</script>

<h1 class="text-4xl font-semibold">{m.nav_permissions()}</h1>
<p class="mt-3 max-w-2xl text-sm text-ink-2">{m.permissions_intro()}</p>

{#if !allowed}
	<p class="mt-8 flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{errorMessage('forbidden')}
	</p>
{:else}
	{#if error}
		<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}

	<section class="mt-8" aria-labelledby="permissions-person">
		<h2 id="permissions-person" class="text-lg font-semibold">{m.permissions_by_person()}</h2>
		<label class="mt-3 block text-sm font-medium" for="permissions-person-select">
			{m.permissions_person()}
		</label>
		<select
			id="permissions-person-select"
			class={field}
			bind:value={personId}
			onchange={showPerson}
		>
			<option value=""></option>
			{#each people as person (person.id)}
				<option value={person.id}>{person.display_name} ({person.username})</option>
			{/each}
		</select>

		{#if byPerson}
			{#if byPerson.administrator}
				<p class="mt-4 flex items-center gap-2 text-sm">
					<ShieldCheck size={16} class="text-ok" aria-hidden="true" />
					{m.permissions_administrator()}
				</p>
			{/if}
			{#if byPerson.groups_from === 'last_sign_in'}
				<p class="mt-4 flex items-center gap-2 text-sm text-ink-2">
					<TriangleAlert size={16} class="text-warning" aria-hidden="true" />
					{m.permissions_groups_last_sign_in()}
				</p>
			{:else if byPerson.groups_from === 'none' && byPerson.user.kind === 'directory'}
				<p class="mt-4 flex items-center gap-2 text-sm text-ink-2">
					<TriangleAlert size={16} class="text-warning" aria-hidden="true" />
					{m.permissions_groups_unknown()}
				</p>
			{/if}
			{#if byPerson.reach.length === 0}
				<p class="mt-4 text-sm text-ink-3">{m.permissions_nothing()}</p>
			{:else}
				<table class="mt-4 w-full text-sm" data-testid="person-report">
					<thead class="text-left text-ink-3">
						<tr>
							<th class={cell}>{m.permissions_object()}</th>
							<th class={cell}>{m.permissions_role()}</th>
							<th class={cell}>{m.permissions_through()}</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-line">
						{#each byPerson.reach as item (item.object.id)}
							<tr>
								<td class={cell}>
									<span class="text-ink-3">{KIND_LABELS[item.object.kind]()}</span>
									<span class="font-medium">{item.name}</span>
									{#if item.path.length > 0}
										<span class="block text-xs text-ink-3">{path(item.path)}</span>
									{/if}
								</td>
								<td class={cell}>{ROLE_LABELS[item.role]()}</td>
								<td class={cell}>
									{#each item.reasons as reason, index (index)}
										<span class="block">
											{reason.inherited
												? m.permissions_reason_inherited({
														role: ROLE_LABELS[reason.role](),
														principal: reason.principal_name,
														on: reason.on_name
													})
												: m.permissions_reason({
														role: ROLE_LABELS[reason.role](),
														principal: reason.principal_name
													})}
										</span>
									{/each}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		{/if}
	</section>

	<section class="mt-10" aria-labelledby="permissions-folder">
		<h2 id="permissions-folder" class="text-lg font-semibold">{m.permissions_by_folder()}</h2>
		<label class="mt-3 block text-sm font-medium" for="permissions-folder-select">
			{m.permissions_folder()}
		</label>
		<select
			id="permissions-folder-select"
			class={field}
			bind:value={folderId}
			onchange={showFolder}
		>
			<option value=""></option>
			{#each folders as folder (folder.id)}
				<option value={folder.id}>{path(folder.path)}</option>
			{/each}
		</select>

		{#if byFolder}
			<p class="mt-4 text-sm text-ink-2">{m.permissions_admins_reach_all()}</p>
			{#if byFolder.holders.length === 0}
				<p class="mt-2 text-sm text-ink-3">{m.permissions_nobody()}</p>
			{:else}
				<table class="mt-4 w-full text-sm" data-testid="folder-report">
					<thead class="text-left text-ink-3">
						<tr>
							<th class={cell}>{m.permissions_who()}</th>
							<th class={cell}>{m.permissions_role()}</th>
							<th class={cell}>{m.permissions_granted_on()}</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-line">
						{#each byFolder.holders as holder, index (index)}
							<tr>
								<td class={cell}>
									<span class="font-medium">{holder.principal_name}</span>
									{#if holder.members.length > 0}
										<span class="block text-xs text-ink-3">
											{m.permissions_members({ names: holder.members.join(', ') })}
										</span>
									{/if}
								</td>
								<td class={cell}>{ROLE_LABELS[holder.role]()}</td>
								<td class={cell}>
									{path(holder.on_path)}
									{#if holder.inherited}
										<span class="block text-xs text-ink-3">{m.permissions_inherited()}</span>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		{/if}
	</section>
{/if}
