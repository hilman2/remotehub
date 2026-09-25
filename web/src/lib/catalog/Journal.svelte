<script lang="ts">
	/** A device's journal (#90): who connected when and why, and the notes left there. */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import NotebookPen from '@lucide/svelte/icons/notebook-pen';
	import Plug from '@lucide/svelte/icons/plug';
	import { errorMessage } from '$lib/api/errors';
	import { addNote, loadJournal, type JournalEntry } from '$lib/api/journal';
	import { formatLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { tabs } from '$lib/session/tabs.svelte';

	let { deviceId }: { deviceId: string } = $props();

	const time = new Intl.DateTimeFormat(formatLocale(), { dateStyle: 'short', timeStyle: 'short' });

	let entries = $state<JournalEntry[] | null>(null);
	let text = $state('');
	let error = $state<string | null>(null);

	async function load(id: string) {
		const result = await loadJournal(id);
		// Another device may have been chosen meanwhile.
		if (id !== deviceId) return;
		if (result.ok) entries = result.data;
		else error = errorMessage(result.code);
	}

	$effect(() => {
		// A session that starts or ends here is a new entry.
		void tabs.list.map((tab) => tab.phase).join();
		error = null;
		load(deviceId);
	});

	async function add(event: SubmitEvent) {
		event.preventDefault();
		if (!text.trim()) return;
		const result = await addNote(deviceId, text);
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		text = '';
		error = null;
		await load(deviceId);
	}

	/** Start and end of a connection, or when a note was left. */
	function when(entry: JournalEntry) {
		const start = new Date(entry.created_at);
		return entry.ended_at ? time.formatRange(start, new Date(entry.ended_at)) : time.format(start);
	}
</script>

<form class="flex items-end gap-2" onsubmit={add}>
	<label class="sr-only" for="note-{deviceId}">{m.journal_note()}</label>
	<textarea
		id="note-{deviceId}"
		class="min-h-10 flex-1 rounded-lg border border-line bg-page px-3 py-2 text-sm"
		rows="2"
		maxlength="2000"
		placeholder={m.journal_note_placeholder()}
		aria-keyshortcuts="Control+Enter Meta+Enter"
		bind:value={text}
		onkeydown={(event) => {
			// Enter alone starts a new line; with Ctrl (Cmd on a Mac) it saves.
			if (event.key !== 'Enter' || !(event.ctrlKey || event.metaKey)) return;
			event.preventDefault();
			event.currentTarget.form?.requestSubmit();
		}}></textarea>
	<button
		type="submit"
		class="h-10 rounded-xl border border-line-strong bg-surface px-3.5 text-sm hover:bg-surface-2 disabled:opacity-50"
		disabled={!text.trim()}
	>
		{m.journal_add_note()}
	</button>
</form>

{#if error}
	<p class="flex items-center gap-2 text-sm" role="alert">
		<CircleAlert size={16} class="text-critical" aria-hidden="true" />
		{error}
	</p>
{/if}

{#if entries?.length === 0}
	<p class="text-sm text-ink-3">{m.journal_empty()}</p>
{:else if entries}
	<ul class="max-h-96 divide-y divide-line overflow-y-auto" aria-label={m.journal_title()}>
		{#each entries as entry (entry.id)}
			<li class="flex gap-3 py-3">
				{#if entry.kind === 'connection'}
					<Plug size={16} class="mt-0.5 shrink-0 text-ink-3" aria-hidden="true" />
					<span class="sr-only">{m.journal_connection()}</span>
				{:else}
					<NotebookPen size={16} class="mt-0.5 shrink-0 text-warning" aria-hidden="true" />
					<span class="sr-only">{m.journal_note()}</span>
				{/if}
				<div class="min-w-0 flex-1">
					<p class="flex flex-wrap items-baseline gap-x-2 text-sm">
						<span class="font-medium" title={entry.username}>{entry.display_name}</span>
						<span class="text-xs text-ink-3">{when(entry)}</span>
						{#if entry.protocol}
							<span class="font-mono text-xs text-ink-3 uppercase">{entry.protocol}</span>
						{/if}
					</p>
					{#if entry.text}
						<p class="mt-0.5 text-sm break-words whitespace-pre-line text-ink-2">{entry.text}</p>
					{/if}
				</div>
			</li>
		{/each}
	</ul>
{/if}
