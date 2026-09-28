<script lang="ts">
	/**
	 * The vault's entries as a table (#193), as KeePass lists them: one row
	 * each, columns sorted by a click on their header. Arrow keys move the
	 * choice; a double click on the user name or password copies it, on the
	 * URL opens it, elsewhere edits the entry. Rows can be dragged onto a
	 * folder of the tree.
	 */
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ChevronUp from '@lucide/svelte/icons/chevron-up';
	import Paperclip from '@lucide/svelte/icons/paperclip';
	import Timer from '@lucide/svelte/icons/timer';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { icon } from './icons';
	import { isExpired, today, type Column, type Item, type Sort } from './items';

	let {
		items,
		sort,
		chosen,
		label,
		onsort,
		onchoose,
		onedit,
		oncopy,
		onopenurl,
		oncontext
	}: {
		items: Item[];
		sort: Sort;
		/** The key of the chosen item. */
		chosen: string | null;
		/** The table's name for assistive technology: where it looks. */
		label: string;
		onsort: (sort: Sort) => void;
		onchoose: (item: Item) => void;
		onedit: (item: Item) => void;
		oncopy: (item: Item, what: 'username' | 'password') => void;
		onopenurl: (item: Item) => void;
		/** A right click or the context menu key, at a point in the window. */
		oncontext: (item: Item, at: { x: number; y: number }) => void;
	} = $props();

	const COLUMNS: { column: Column | 'password'; label: () => string }[] = [
		{ column: 'title', label: m.field_title },
		{ column: 'username', label: m.field_username },
		{ column: 'password', label: m.field_password },
		{ column: 'url', label: m.field_url },
		{ column: 'notes', label: m.field_notes },
		{ column: 'changed', label: m.vault_column_changed }
	];

	const day = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short' });
	const now = today();

	let list = $state<HTMLUListElement>();

	function pick(column: Column) {
		onsort({ column, descending: sort.column === column ? !sort.descending : false });
	}

	/** Arrow keys move the choice, as in a list box; the page does the rest. */
	function onkeydown(event: KeyboardEvent) {
		if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
		event.preventDefault();
		const at = items.findIndex((item) => item.key === chosen);
		const next =
			event.key === 'ArrowDown' ? Math.min(at + 1, items.length - 1) : Math.max(at - 1, 0);
		const item = items[next];
		if (!item) return;
		onchoose(item);
		list?.querySelectorAll<HTMLElement>('[role=row]')[next]?.focus();
	}

	function context(event: MouseEvent | KeyboardEvent, item: Item) {
		event.preventDefault();
		onchoose(item);
		if (event instanceof MouseEvent && event.clientX) {
			oncontext(item, { x: event.clientX, y: event.clientY });
		} else {
			const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
			oncontext(item, { x: box.left + 24, y: box.bottom });
		}
	}

	const grid =
		'grid grid-cols-[minmax(10rem,1.4fr)_minmax(8rem,1fr)_6rem_minmax(8rem,1.2fr)_minmax(6rem,1fr)_6.5rem] items-center';
	const cell = 'truncate px-2';
</script>

<div
	role="grid"
	aria-label={label}
	aria-rowcount={items.length}
	class="flex min-h-0 flex-1 flex-col"
>
	<div
		role="row"
		class="{grid} h-9 shrink-0 border-b border-line bg-surface-2 px-2 text-xs font-semibold text-ink-2"
	>
		{#each COLUMNS as { column, label: text } (column)}
			<span
				role="columnheader"
				aria-sort={sort.column === column ? (sort.descending ? 'descending' : 'ascending') : 'none'}
			>
				{#if column === 'password'}
					<span class="px-2">{text()}</span>
				{:else}
					<button
						type="button"
						class="flex w-full items-center gap-1 rounded px-2 py-1 text-left hover:bg-surface"
						onclick={() => pick(column)}
					>
						{text()}
						{#if sort.column === column}
							{#if sort.descending}
								<ChevronDown size={13} aria-hidden="true" />
							{:else}
								<ChevronUp size={13} aria-hidden="true" />
							{/if}
						{/if}
					</button>
				{/if}
			</span>
		{/each}
	</div>
	<ul bind:this={list} role="rowgroup" class="relative min-h-0 flex-1 overflow-y-auto">
		{#each items as item (item.key)}
			{@const Icon = icon(item.icon)}
			{@const expired = isExpired(item, now)}
			<li
				role="row"
				tabindex={item.key === chosen || (!chosen && item === items[0]) ? 0 : -1}
				aria-selected={item.key === chosen}
				data-testid={item.source === 'shared' ? 'shared-entry' : 'personal-entry'}
				draggable="true"
				class="{grid} h-9 cursor-default border-b border-line/60 px-2 text-sm outline-none hover:bg-surface-2 focus-visible:ring-2 focus-visible:ring-accent aria-selected:bg-accent/15"
				onclick={() => onchoose(item)}
				ondblclick={() => onedit(item)}
				oncontextmenu={(event) => context(event, item)}
				onkeydown={(event) => {
					if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
						context(event, item);
					} else onkeydown(event);
				}}
				ondragstart={(event) => {
					event.dataTransfer?.setData('text/x-remotehub-entry', item.key);
					if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
				}}
			>
				<span role="gridcell" class="flex min-w-0 items-center gap-2 px-2">
					<Icon size={15} class="shrink-0 text-warning" aria-hidden="true" />
					<span class="truncate font-medium">{item.title}</span>
					{#if item.hasTotp}
						<Timer size={13} class="shrink-0 text-ink-3" aria-label={m.vault_has_totp()} />
					{/if}
					{#if item.files > 0}
						<Paperclip size={13} class="shrink-0 text-ink-3" aria-label={m.vault_has_files()} />
					{/if}
				</span>
				<span
					role="gridcell"
					tabindex="-1"
					class="{cell} font-mono text-xs"
					ondblclick={(event) => {
						event.stopPropagation();
						oncopy(item, 'username');
					}}
				>
					{item.username}
				</span>
				<span
					role="gridcell"
					tabindex="-1"
					class="{cell} font-mono text-ink-3"
					ondblclick={(event) => {
						event.stopPropagation();
						oncopy(item, 'password');
					}}
				>
					<span aria-hidden="true">••••••••</span>
					<span class="sr-only">{m.credential_hidden()}</span>
				</span>
				<span
					role="gridcell"
					tabindex="-1"
					class="{cell} text-accent"
					ondblclick={(event) => {
						event.stopPropagation();
						if (item.url) onopenurl(item);
					}}
				>
					{item.url.replace(/^https?:\/\//i, '')}
				</span>
				<span role="gridcell" class="{cell} text-ink-2">{item.notes.split('\n')[0]}</span>
				<span role="gridcell" class="{cell} text-xs text-ink-2 tabular-nums">
					{#if expired}
						<span class="inline-flex items-center gap-1 font-medium text-warning">
							<TriangleAlert size={12} aria-hidden="true" />
							{m.vault_expired()}
						</span>
					{:else if item.changed}
						{day.format(new Date(item.changed))}
					{/if}
				</span>
			</li>
		{:else}
			<li class="px-4 py-6 text-sm text-ink-3">{m.vault_empty()}</li>
		{/each}
	</ul>
</div>
