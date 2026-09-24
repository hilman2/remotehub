<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import FolderClosed from '@lucide/svelte/icons/folder-closed';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import Globe from '@lucide/svelte/icons/globe';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Monitor from '@lucide/svelte/icons/monitor';
	import SquareTerminal from '@lucide/svelte/icons/square-terminal';
	import type { ObjectKind } from '$lib/api/catalog';
	import { m } from '$lib/paraglide/messages';
	import FolderNodeView from './FolderNodeView.svelte';
	import type { FolderNode } from './tree';

	let {
		node,
		depth = 0,
		expanded,
		selected,
		onselect,
		ontoggle
	}: {
		node: FolderNode;
		depth?: number;
		expanded: (id: string) => boolean;
		selected: { kind: ObjectKind; id: string } | null;
		onselect: (kind: ObjectKind, id: string) => void;
		ontoggle: (id: string) => void;
	} = $props();

	const open = $derived(expanded(node.folder.id));
	const isSelected = (kind: ObjectKind, id: string) =>
		selected?.kind === kind && selected.id === id;
	const row =
		'flex w-full min-w-0 items-center gap-2 rounded-md py-1 pl-1 pr-2 text-left text-sm hover:bg-surface-2 data-[selected=true]:bg-surface-2 data-[selected=true]:font-medium';
	const indent = (level: number) => `padding-left: ${0.5 + level * 1.1}rem`;
</script>

<li role="treeitem" aria-expanded={open} aria-selected={isSelected('folder', node.folder.id)}>
	<div class="flex items-center" style={indent(depth)}>
		<button
			type="button"
			class="rounded p-0.5 text-ink-3 hover:text-ink"
			title={open ? m.tree_collapse() : m.tree_expand()}
			onclick={() => ontoggle(node.folder.id)}
		>
			<ChevronRight
				size={14}
				class={open ? 'rotate-90 transition-transform' : 'transition-transform'}
				aria-hidden="true"
			/>
			<span class="sr-only">{open ? m.tree_collapse() : m.tree_expand()}</span>
		</button>
		<button
			type="button"
			class={row}
			data-selected={isSelected('folder', node.folder.id)}
			onclick={() => onselect('folder', node.folder.id)}
			ondblclick={() => ontoggle(node.folder.id)}
		>
			{#if open}
				<FolderOpen size={16} class="shrink-0 text-accent" aria-hidden="true" />
			{:else}
				<FolderClosed size={16} class="shrink-0 text-accent" aria-hidden="true" />
			{/if}
			<span class="truncate" class:text-ink-2={node.folder.role === null}>{node.folder.name}</span>
		</button>
	</div>

	{#if open}
		<ul role="group">
			{#each node.folders as child (child.folder.id)}
				<FolderNodeView
					node={child}
					depth={depth + 1}
					{expanded}
					{selected}
					{onselect}
					{ontoggle}
				/>
			{/each}
			{#each node.devices as device (device.id)}
				<li role="treeitem" aria-selected={isSelected('device', device.id)}>
					<button
						type="button"
						class={row}
						style={indent(depth + 1.4)}
						data-selected={isSelected('device', device.id)}
						onclick={() => onselect('device', device.id)}
					>
						{#if device.protocol === 'ssh'}
							<SquareTerminal size={16} class="shrink-0 text-ink-2" aria-hidden="true" />
						{:else if device.protocol === 'https'}
							<Globe size={16} class="shrink-0 text-ink-2" aria-hidden="true" />
						{:else}
							<Monitor size={16} class="shrink-0 text-ink-2" aria-hidden="true" />
						{/if}
						<span class="truncate">{device.name}</span>
						<span class="ml-auto truncate font-mono text-xs text-ink-3">{device.host}</span>
					</button>
				</li>
			{/each}
			{#each node.credentials as credential (credential.id)}
				<li role="treeitem" aria-selected={isSelected('credential', credential.id)}>
					<button
						type="button"
						class={row}
						style={indent(depth + 1.4)}
						data-selected={isSelected('credential', credential.id)}
						onclick={() => onselect('credential', credential.id)}
					>
						<KeyRound size={16} class="shrink-0 text-warning" aria-hidden="true" />
						<span class="truncate">{credential.name}</span>
						<span class="ml-auto truncate text-xs text-ink-3">{credential.username}</span>
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</li>
