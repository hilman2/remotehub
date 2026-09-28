<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import FolderClosed from '@lucide/svelte/icons/folder-closed';
	import FolderInput from '@lucide/svelte/icons/folder-input';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import { allows, type ObjectKind } from '$lib/api/catalog';
	import { m } from '$lib/paraglide/messages';
	import FolderNodeView from './FolderNodeView.svelte';
	import ProtocolChip from './ProtocolChip.svelte';
	import type { Dragged, FolderNode, Mover } from './tree';

	let {
		node,
		depth = 0,
		expanded,
		selected,
		onselect,
		ontoggle,
		onopen,
		mover
	}: {
		node: FolderNode;
		depth?: number;
		expanded: (id: string) => boolean;
		selected: { kind: ObjectKind; id: string } | null;
		onselect: (kind: ObjectKind, id: string) => void;
		ontoggle: (id: string) => void;
		/** A device was double-clicked. */
		onopen: (deviceId: string) => void;
		mover: Mover;
	} = $props();

	const open = $derived(expanded(node.folder.id));
	const isSelected = (kind: ObjectKind, id: string) =>
		selected?.kind === kind && selected.id === id;
	const row =
		'flex w-full min-w-0 items-center gap-2.5 rounded-lg pl-1.5 pr-2.5 text-left text-sm text-ink-2 hover:bg-surface-2 hover:text-ink data-[selected=true]:bg-surface-2 data-[selected=true]:text-ink data-[selected=true]:ring-1 data-[selected=true]:ring-line-strong';
	const indent = (level: number) => `padding-left: ${0.5 + level * 1.1}rem`;

	/** Something is dragged over this folder, and the folder takes it (#214). */
	let over = $state(false);

	function dragStart(event: DragEvent, item: Dragged, name: string) {
		if (!event.dataTransfer) return;
		event.dataTransfer.effectAllowed = 'move';
		// Other programs get the name, nothing more.
		event.dataTransfer.setData('text/plain', name);
		mover.start(item);
	}

	function dragOver(event: DragEvent) {
		if (!mover.takes(node.folder.id)) return;
		event.preventDefault();
		if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
		over = true;
	}

	function dragLeave(event: DragEvent) {
		// Leaving for one of the row's own buttons is no leaving.
		const to = event.relatedTarget;
		if (!(to instanceof Node && event.currentTarget instanceof Node)) over = false;
		else if (!event.currentTarget.contains(to)) over = false;
	}

	function drop(event: DragEvent) {
		event.preventDefault();
		over = false;
		mover.drop(node.folder.id);
	}
</script>

<li role="treeitem" aria-expanded={open} aria-selected={isSelected('folder', node.folder.id)}>
	<div
		class="flex items-center rounded-lg"
		class:drop-target={over}
		style={indent(depth)}
		ondragover={dragOver}
		ondragleave={dragLeave}
		ondrop={drop}
	>
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
			class="{row} py-1.5"
			data-selected={isSelected('folder', node.folder.id)}
			draggable={allows(node.folder.role, 'manage')}
			onclick={() => onselect('folder', node.folder.id)}
			ondblclick={() => ontoggle(node.folder.id)}
			ondragstart={(event) =>
				dragStart(event, { kind: 'folder', id: node.folder.id }, node.folder.name)}
			ondragend={mover.end}
		>
			{#if open}
				<FolderOpen size={16} class="shrink-0 text-ink-3" aria-hidden="true" />
			{:else}
				<FolderClosed size={16} class="shrink-0 text-ink-3" aria-hidden="true" />
			{/if}
			<span class="truncate" class:text-ink-3={node.folder.role === null}>{node.folder.name}</span>
			{#if over}
				<!-- Where it goes, not by colour alone (#214). -->
				<FolderInput size={15} class="ml-auto shrink-0 text-accent" aria-hidden="true" />
			{/if}
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
					{onopen}
					{mover}
				/>
			{/each}
			{#each node.devices as device (device.id)}
				<li role="treeitem" aria-selected={isSelected('device', device.id)}>
					<button
						type="button"
						class="{row} py-1"
						style={indent(depth + 1.4)}
						data-selected={isSelected('device', device.id)}
						draggable={allows(device.role, 'edit')}
						onclick={() => onselect('device', device.id)}
						ondblclick={() => onopen(device.id)}
						ondragstart={(event) => dragStart(event, { kind: 'device', id: device.id }, device.name)}
						ondragend={mover.end}
					>
						<ProtocolChip protocol={device.protocol} />
						<!-- The name gets the whole line, the host goes small below it (#209). -->
						<span class="flex min-w-0 flex-col">
							<span class="truncate">{device.name}</span>
							<span class="truncate font-mono text-[11px] leading-tight text-ink-3">
								{device.host}
							</span>
						</span>
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</li>

<style>
	.drop-target {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
		background: var(--surface-2);
	}
</style>
