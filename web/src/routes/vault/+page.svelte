<script lang="ts">
	/**
	 * The vault (#190, #193), laid out as KeePass' main window: a toolbar, the
	 * folder tree with the personal and the shared vault and their recycle
	 * bins, the entries as a table, the chosen entry below it and a status
	 * bar. Shortcuts, a context menu and dragging onto a folder do what the
	 * toolbar does. Shared collections work without the personal vault; that
	 * one opens in this browser only.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Clock from '@lucide/svelte/icons/clock';
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import FileDown from '@lucide/svelte/icons/file-down';
	import FileUp from '@lucide/svelte/icons/file-up';
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderInput from '@lucide/svelte/icons/folder-input';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Layers from '@lucide/svelte/icons/layers';
	import Lock from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import User from '@lucide/svelte/icons/user';
	import Users from '@lucide/svelte/icons/users';
	import {
		allows,
		createCredential,
		deleteCollection,
		deleteCredential,
		loadTree,
		restoreCredential,
		updateCredential,
		type Collection,
		type Credential,
		type Role,
		type Tree
	} from '$lib/api/catalog';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import { createRequest } from '$lib/api/requests';
	import { credentialCode, deleteAttachment, reveal, uploadAttachment } from '$lib/api/reveal';
	import { loadPicks, savePick } from '$lib/api/search';
	import Grants from '$lib/catalog/Grants.svelte';
	import RequestForm from '$lib/catalog/RequestForm.svelte';
	import { collectionPath } from '$lib/catalog/tree';
	import ContextMenu, { type ContextItem } from '$lib/components/ContextMenu.svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import SettingsMenu, { type MenuItem } from '$lib/components/SettingsMenu.svelte';
	import { getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { pickKey } from '$lib/search/catalog';
	import { queryKey, remember, type Pick } from '$lib/search/rank';
	import { session } from '$lib/session.svelte';
	import {
		contentOf,
		draftOf,
		inputFromCredential,
		inputOf,
		newDraft,
		personalAsKdbx,
		placeOf,
		purgePersonal,
		savePersonalKdbx,
		targetOf,
		type Target
	} from '$lib/vault/actions';
	import CollectionForm from '$lib/vault/CollectionForm.svelte';
	import EntryEditor, { type Draft, type Place } from '$lib/vault/EntryEditor.svelte';
	import EntryPane from '$lib/vault/EntryPane.svelte';
	import EntryTable from '$lib/vault/EntryTable.svelte';
	import { FOLDER_ICON } from '$lib/vault/icons';
	import {
		below,
		counter,
		listed,
		outline,
		personalFolders,
		personalItems,
		personalTotp,
		sharedItems,
		type Item,
		type Scope,
		type Sort
	} from '$lib/vault/items';
	import KdbxForm from '$lib/vault/KdbxForm.svelte';
	import { writeKdbx, type KdbxEntry } from '$lib/vault/kdbx';
	import { credentialAsKdbx, exportCollection, importInto } from '$lib/vault/shared-kdbx';
	import { totpCode, totpOf } from '$lib/vault/totp';
	import { unlocked } from '$lib/vault/unlocked.svelte';
	import {
		addPasskey,
		asKdbx,
		coverForOrganisation,
		deleteFile,
		loadVault,
		MAX_FILE,
		oneTimeRecovery,
		passkeysAvailable,
		readEntries,
		readFile,
		readPicks,
		removeUnlock,
		renewRecovery,
		resetVault,
		saveEntry,
		saveFile,
		savePassphrase,
		savePicks,
		setUp,
		unlockWithPasskey,
		unlockWithPassphrase,
		unlockWithRecovery,
		withHistory,
		type Entry,
		type EntryContent,
		type FileRef,
		type StoredVault,
		type UnlockKind
	} from '$lib/vault/vault';

	type Stage = 'loading' | 'setup' | 'recovery' | 'locked' | 'open';

	const MIN_PASSPHRASE = 12;
	/** How long a copied secret stays in the clipboard, in seconds. */
	const CLIPBOARD_SECONDS = 30;
	const KIND_LABELS: Record<UnlockKind, () => string> = {
		passkey: m.vault_kind_passkey,
		passphrase: m.vault_kind_passphrase,
		recovery: m.vault_kind_recovery,
		organisation: m.vault_kind_organisation
	};
	const EMPTY: EntryContent = { title: '', username: '', password: '', url: '', notes: '' };
	const SORT_KEY = 'remotehub.vault.sort';
	const DRAG_TYPE = 'text/x-remotehub-entry';
	/** A personal folder, dragged into Shared (#218). */
	const FOLDER_DRAG_TYPE = 'text/x-remotehub-folder';
	/** The place of Shared itself, for a folder dropped at its top. */
	const SHARED_TOP = 'shared-top';

	// ---- The personal vault ----

	let stage = $state<Stage>('loading');
	let vault = $state<StoredVault | null>(null);
	// The vault key: in memory only, kept in `unlocked` while the page is
	// loaded, so it is still open after a visit to another page.
	let key: CryptoKey | null = unlocked.key;
	let entries = $state<Entry[]>([]);
	let error = $state<string | null>(null);
	let busy = $state(false);

	let passphrase = $state('');
	let passphraseAgain = $state('');
	let recoveryText = $state('');
	let useRecovery = $state(false);
	let shownRecovery = $state('');
	let passkeyLabel = $state('');
	/** After a one-time recovery key: the new passphrase comes next. */
	let mustRenew = $state(false);

	/** What the owner used; personal ones sealed in the vault (#81). */
	let picks = $state<Pick[]>([]);

	// ---- The shared collections ----

	let tree = $state<Tree | null>(null);
	/** Picks of shared credentials, kept on the server like the devices page's. */
	let sharedPicks = $state<Pick[]>([]);

	// ---- What is shown ----

	let query = $state('');
	let search = $state<HTMLInputElement>();
	let scope = $state<Scope>({ kind: 'all' });
	let sort = $state<Sort>(readSort());
	/** The key of the item chosen (Item.key). */
	let chosen = $state<string | null>(null);
	/** A line in the status bar, and how long a copied secret still stays. */
	let status = $state<string | null>(null);
	let clearing = $state<{ left: number; value: string } | null>(null);
	let menu = $state<{ item: Item; at: { x: number; y: number } } | null>(null);
	/** The folder a dragged entry is over, as its place. */
	let dropOver = $state<string | null>(null);

	function readSort(): Sort {
		try {
			const stored = JSON.parse(localStorage.getItem(SORT_KEY) ?? 'null');
			if (stored && typeof stored.column === 'string') return stored as Sort;
		} catch {
			// Without storage, the default is fine.
		}
		return { column: 'title', descending: false };
	}

	$effect(() => {
		try {
			localStorage.setItem(SORT_KEY, JSON.stringify(sort));
		} catch {
			// A preference only.
		}
	});

	const folders = $derived(personalFolders(entries));
	const items = $derived([...(tree ? sharedItems(tree) : []), ...personalItems(entries)]);
	const allPicks = $derived([...picks, ...sharedPicks]);
	/** Picks fade with age; the page's lifetime is short enough to hold one time. */
	const now = Date.now();
	const searching = $derived(queryKey(query).length > 0);
	const shown = $derived(
		listed(items, {
			scope,
			sort,
			query,
			picks: allPicks,
			now,
			folders,
			collections: tree?.collections ?? [],
			locale: getLocale()
		})
	);
	const count = $derived(
		counter(items, { folders, collections: tree?.collections ?? [], picks: allPicks, now })
	);
	const current = $derived(items.find((item) => item.key === chosen) ?? null);
	const personalOutline = $derived(outline(folders, getLocale()));
	const collectionOutline = $derived(outline(tree?.collections ?? [], getLocale()));
	/** Entries that do not open with this key; they can only be deleted. */
	const unreadable = $derived(entries.filter((entry) => !entry.content));

	const scopeCollection = $derived.by(() => {
		const here = scope;
		return here.kind === 'collection' ? tree?.collections.find((c) => c.id === here.id) : undefined;
	});
	const scopeFolder = $derived.by(() => {
		const here = scope;
		return here.kind === 'personal' && here.folder
			? folders.find((f) => f.id === here.folder)
			: undefined;
	});
	/** The personal vault is shown but not open: its setup or unlock form takes the table's place. */
	const staging = $derived(
		!searching &&
			stage !== 'open' &&
			(scope.kind === 'personal' || (scope.kind === 'bin' && scope.side === 'personal'))
	);

	/** Where entries may go, for the editor: the personal folders and the collections one may edit. */
	const places = $derived.by((): Place[] => {
		const personal: Place[] =
			stage === 'open'
				? [
						{
							value: placeOf({ side: 'personal', folder: null }),
							label: m.vault_personal_top(),
							group: 'personal'
						},
						...personalOutline.map((node) => ({
							value: placeOf({ side: 'personal', folder: node.id }),
							label: `${'  '.repeat(node.depth)}${node.name}`,
							group: 'personal' as const
						}))
					]
				: [];
		const shared: Place[] = tree
			? collectionOutline
					.filter((node) =>
						allows(tree!.collections.find((c) => c.id === node.id)?.role ?? null, 'edit')
					)
					.map((node) => ({
						value: placeOf({ side: 'shared', id: node.id }),
						label: collectionPath(tree!, node.id)
							.map((c) => c.name)
							.join(' / '),
						group: 'shared' as const
					}))
			: [];
		return [...personal, ...shared];
	});

	const sameScope = (a: Scope, b: Scope) => JSON.stringify(a) === JSON.stringify(b);

	const scopeTitle = $derived.by(() => {
		if (searching) return m.vault_search_title();
		switch (scope.kind) {
			case 'all':
				return m.vault_all();
			case 'recent':
				return m.vault_recent();
			case 'expiring':
				return m.vault_expiring();
			case 'personal':
				return scopeFolder?.name ?? m.vault_personal();
			case 'collection':
				return scopeCollection?.name ?? '';
			case 'bin':
				return scope.side === 'personal' ? m.vault_bin_personal() : m.vault_bin_shared();
		}
	});

	// ---- Loading ----

	async function loadPersonal() {
		const result = await loadVault();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		vault = result.data;
		if (key && (await coverForOrganisation(key, vault))) {
			const again = await loadVault();
			if (again.ok) vault = again.data;
		}
		if (key) {
			[entries, picks] = await Promise.all([readEntries(key, vault), readPicks(key, vault)]);
			if (stage === 'loading') stage = 'open';
		} else stage = vault.unlocks.length === 0 ? 'setup' : 'locked';
	}

	async function loadShared() {
		const [result, picked] = await Promise.all([loadTree(), loadPicks()]);
		if (result.ok) tree = result.data;
		else error = errorMessage(result.code);
		if (picked.ok) sharedPicks = picked.data.filter((pick) => pick.key.startsWith('credential:'));
	}

	const reload = () => Promise.all([loadPersonal(), loadShared()]);

	$effect(() => {
		reload();
	});

	// ---- Choosing ----

	function pick(next: Scope) {
		scope = next;
		query = '';
	}

	/** Chooses an item and remembers the pick for the current query. */
	function choose(item: Item) {
		if (chosen === item.key) return;
		chosen = item.key;
		used(item);
	}

	function used(item: Item) {
		if (item.source === 'personal') {
			if (!key) return;
			picks = remember(picks, item.key, query, Date.now());
			// A preference: if it is not stored, the vault still works.
			savePicks(key, picks);
		} else {
			sharedPicks = remember(sharedPicks, item.key, query, Date.now());
			savePick(item.key, queryKey(query));
		}
	}

	const mayEdit = (item: Item) =>
		item.source === 'personal' || allows(item.credential.role, 'edit');
	const mayReveal = (item: Item) =>
		item.source === 'personal' || allows(item.credential.role, 'reveal');

	// ---- Clipboard ----

	$effect(() => {
		if (!clearing) return;
		const timer = setInterval(async () => {
			if (!clearing) return;
			if (clearing.left > 1) {
				clearing = { ...clearing, left: clearing.left - 1 };
				return;
			}
			const copied = clearing.value;
			clearing = null;
			status = m.vault_clipboard_cleared();
			// Only what is still the copied value goes. A browser that does not
			// let the page read the clipboard gets it cleared all the same.
			let now: string;
			try {
				now = await navigator.clipboard.readText();
			} catch {
				now = copied;
			}
			if (now === copied) await navigator.clipboard.writeText('').catch(() => {});
		}, 1000);
		return () => clearInterval(timer);
	});

	/** Copies what an entry holds; secrets leave the clipboard after a while. */
	async function copy(item: Item, what: 'username' | 'password' | 'totp') {
		let value: string | null;
		if (what === 'username') value = item.username;
		else if (!mayReveal(item)) return;
		else if (item.source === 'personal') {
			if (what === 'password') value = item.entry.content?.password ?? '';
			else {
				const params = totpOf('otp', personalTotp(item.entry));
				value = params ? await totpCode(params, Date.now() / 1000) : null;
			}
		} else if (what === 'password') {
			const result = await reveal('credentials', item.id, 'copy');
			if (!result.ok) return fail(result.code);
			value = result.data.password ?? '';
		} else {
			const result = await credentialCode(item.id, 'copy');
			if (!result.ok) return fail(result.code);
			value = result.data.code;
		}
		if (value === null) return;
		try {
			await navigator.clipboard.writeText(value);
		} catch {
			status = m.reveal_copy_failed();
			return;
		}
		used(item);
		const what_ = {
			username: m.field_username,
			password: m.field_password,
			totp: m.vault_totp
		}[what]();
		if (what === 'username') {
			clearing = null;
			status = m.vault_copied_what({ what: what_, title: item.title });
		} else {
			clearing = { left: CLIPBOARD_SECONDS, value };
			status = m.vault_copied_what({ what: what_, title: item.title });
		}
	}

	function fail(code: string) {
		status = errorMessage(code);
	}

	function openUrl(item: Item) {
		if (!/^https?:\/\//i.test(item.url)) return;
		window.open(item.url, '_blank', 'noopener,noreferrer');
		used(item);
	}

	// ---- Setting up and unlocking the personal vault ----

	function passphraseProblem(): string | null {
		if (passphrase.length < MIN_PASSPHRASE)
			return m.vault_passphrase_short({ count: MIN_PASSPHRASE });
		if (passphrase !== passphraseAgain) return m.vault_passphrase_mismatch();
		return null;
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		error = passphraseProblem();
		if (error) return;
		busy = true;
		const result = await setUp(passphrase);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		key = unlocked.key = result.key;
		shownRecovery = result.recovery;
		passphrase = passphraseAgain = '';
		stage = 'recovery';
	}

	async function opened(opening: CryptoKey | null) {
		if (!opening || !vault) {
			error = m.vault_wrong();
			return;
		}
		key = unlocked.key = opening;
		error = null;
		passphrase = recoveryText = '';
		// A one-time key from a recovery (#95) is replaced at once, and the
		// forgotten passphrase with it.
		if (useRecovery && oneTimeRecovery(vault)) {
			mustRenew = true;
			useRecovery = false;
			await newRecovery();
			return;
		}
		// Reads the entries, and wraps the key for the organisation if needed.
		await loadPersonal();
		stage = 'open';
	}

	async function unlock(event: SubmitEvent) {
		event.preventDefault();
		if (!vault) return;
		busy = true;
		const opening = useRecovery
			? await unlockWithRecovery(vault, recoveryText)
			: await unlockWithPassphrase(vault, passphrase);
		busy = false;
		await opened(opening);
	}

	async function unlockPasskey() {
		if (!vault) return;
		try {
			await opened(await unlockWithPasskey(vault));
		} catch {
			error = m.vault_wrong();
		}
	}

	function lock() {
		if (stage !== 'open') return;
		key = unlocked.key = null;
		entries = [];
		picks = [];
		if (current?.source === 'personal') chosen = null;
		stage = 'locked';
		status = m.vault_locked_status();
	}

	async function afterRecovery() {
		await loadPersonal();
		if (key && vault) entries = await readEntries(key, vault);
		stage = 'open';
		if (mustRenew) {
			mustRenew = false;
			show({ type: 'passphrase' });
		}
	}

	// ---- Dialogs ----

	type Open =
		| { type: 'entry'; item: Item | null; draft: Draft }
		| { type: 'folder' }
		| { type: 'collection'; collection: Collection | null; parent: string | null }
		| { type: 'purge'; item: Item }
		| { type: 'empty'; side: 'personal' | 'shared' }
		| { type: 'request'; credential: Credential; role: Role }
		/** `shared`: into Shared itself, at the top (#216); else a collection, or the personal vault. */
		| { type: 'kdbx'; mode: 'import' | 'export'; collection: Collection | null; shared?: true }
		| { type: 'move-folder'; folder: string; name: string }
		| { type: 'unlocks' }
		| { type: 'passphrase' }
		| { type: 'reset' };

	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let kdbxResult = $state<string | null>(null);
	let folderName = $state('');
	/** Where *Move to Shared* puts a personal folder: a collection's id, or `top` (#218). */
	let moveInto = $state('');
	/** The places in Shared a personal folder may go, with their paths. */
	const sharedPlaces = $derived.by(() => {
		if (!tree) return [];
		const known = tree;
		const places = known.collections
			.filter((c) => allows(c.role, 'manage'))
			.map((c) => ({
				id: c.id,
				path: collectionPath(known, c.id)
					.map((p) => p.name)
					.join(' / ')
			}))
			.sort((a, b) => a.path.localeCompare(b.path, getLocale(), { sensitivity: 'base' }));
		return known.may_create_top_level ? [{ id: 'top', path: '' }, ...places] : places;
	});

	function show(next: Open) {
		error = null;
		kdbxResult = null;
		menu = null;
		moveInto = '';
		open = next;
		dialogOpen = true;
	}

	function close() {
		dialogOpen = false;
		error = null;
	}

	const dialogTitle = $derived.by(() => {
		switch (open?.type) {
			case 'entry':
				return open.item ? m.vault_edit_entry() : m.vault_new_entry();
			case 'folder':
				return m.vault_new_folder();
			case 'collection':
				return open.collection ? m.vault_manage_collection() : m.vault_new_collection();
			case 'purge':
				return m.vault_purge();
			case 'empty':
				return m.vault_empty_bin();
			case 'request':
				return m.request_title({ name: open.credential.name });
			case 'kdbx':
				if (open.shared) return m.kdbx_import_shared();
				return open.mode === 'import' ? m.kdbx_import() : m.kdbx_export();
			case 'move-folder':
				return m.vault_move_folder_title({ name: open.name });
			case 'unlocks':
				return m.vault_unlocks_title();
			case 'passphrase':
				return m.vault_change_passphrase();
			case 'reset':
				return m.vault_reset();
			default:
				return '';
		}
	});

	// ---- Entries: new, edit, save ----

	/** Where "new" puts an entry: the folder shown, or else a place one may write to. */
	function newPlace(): string | null {
		if (scope.kind === 'personal' && stage === 'open') {
			return placeOf({ side: 'personal', folder: scope.folder });
		}
		if (scope.kind === 'collection') {
			const here = placeOf({ side: 'shared', id: scope.id });
			if (places.some((p) => p.value === here)) return here;
		}
		return places[0]?.value ?? null;
	}

	function newEntry() {
		const place = newPlace();
		if (place) show({ type: 'entry', item: null, draft: newDraft(place) });
	}

	function edit(item: Item) {
		if (item.deleted || !mayEdit(item)) return;
		used(item);
		show({ type: 'entry', item, draft: draftOf(item) });
	}

	async function saveDraft(draft: Draft) {
		if (open?.type !== 'entry') return;
		const original = open.item;
		const target = targetOf(draft.place);
		if (draft.newFiles.some((file) => file.size > MAX_FILE)) {
			error = m.vault_file_too_large({ megabytes: MAX_FILE / 1024 / 1024 });
			return;
		}
		busy = true;
		error = null;
		// Saved where it is first; a move to the other side follows.
		const here: Target = original
			? original.source === 'personal'
				? target.side === 'personal'
					? target
					: { side: 'personal', folder: original.folder }
				: target.side === 'shared'
					? target
					: { side: 'shared', id: original.collection }
			: target;
		const saved = await saveHere(draft, original, here);
		if (saved && here.side !== target.side) {
			await reload();
			const moved = items.find((item) => item.key === saved);
			if (moved) await move(moved, target);
		}
		busy = false;
		if (saved) {
			close();
			await reload();
			if (here.side === target.side) chosen = saved;
		}
	}

	/** Saves a draft on its side; the key of the saved item, or null. */
	async function saveHere(
		draft: Draft,
		original: Item | null,
		here: Target
	): Promise<string | null> {
		if (here.side === 'personal') {
			if (!key) return null;
			const added: FileRef[] = [];
			for (const file of draft.newFiles) {
				const stored = await saveFile(key, file);
				if (!stored) {
					error = errorMessage('internal');
					return null;
				}
				added.push(stored);
			}
			const before = original?.source === 'personal' ? original.entry.content : null;
			let content = contentOf(draft, here.folder, before, added);
			if (before) content = withHistory(before, content);
			const result = await saveEntry(key, original?.id ?? null, content);
			if (!result.ok) {
				error = errorMessage(result.code);
				return null;
			}
			for (const id of draft.removedFiles) await deleteFile(id);
			return result.id;
		}
		const input = inputOf(draft, here.id, !original);
		const result = original
			? await updateCredential(original.id, input)
			: await createCredential(input);
		if (!result.ok) {
			error = problemMessage(result);
			return null;
		}
		const id = original?.id ?? (result.data as { id: string }).id;
		for (const file of draft.newFiles) await uploadAttachment(id, file);
		for (const file of draft.removedFiles) await deleteAttachment(id, file);
		return pickKey('credential', id);
	}

	// ---- Moving, copying, deleting ----

	/**
	 * Moves an entry to another folder. Across the personal and the shared
	 * side it is written anew there and deleted here, as KeePass would move it
	 * between two files.
	 */
	async function move(item: Item, target: Target): Promise<boolean> {
		if (!mayEdit(item)) return false;
		if (item.source === 'personal' && target.side === 'personal') {
			if (!key || !item.entry.content || item.folder === target.folder) return false;
			const result = await saveEntry(key, item.id, {
				...item.entry.content,
				parent: target.folder
			});
			return result.ok || (fail(result.code), false);
		}
		if (item.source === 'shared' && target.side === 'shared') {
			if (item.collection === target.id) return false;
			const result = await updateCredential(
				item.id,
				inputFromCredential(item.credential, target.id)
			);
			return result.ok || (fail(result.code), false);
		}
		if (item.source === 'personal' && target.side === 'shared') {
			if (!key) return false;
			const entry = await personalAsKdbx(key, item.entry);
			if (!tree || !entry) return false;
			const made = await importInto(tree, target.id, [entry]);
			if (made.created !== 1) return ((status = m.vault_move_failed({ title: item.title })), false);
			await purgePersonal(item.entry);
			chosen = pickKey('credential', made.ids[0]);
			return true;
		}
		if (item.source === 'shared' && target.side === 'personal') {
			if (!key || !tree || !mayReveal(item)) return false;
			const entry = await credentialAsKdbx(tree, item.credential);
			if (!entry) return ((status = m.vault_move_failed({ title: item.title })), false);
			const id = await savePersonalKdbx(key, { ...entry, path: [] }, target.folder);
			if (!id) return ((status = m.vault_move_failed({ title: item.title })), false);
			// Moved, not copied.
			await deleteCredential(item.id, true);
			chosen = id;
			return true;
		}
		return false;
	}

	async function moveTo(item: Item, target: Target) {
		busy = true;
		const moved = await move(item, target);
		busy = false;
		if (moved) {
			status = m.vault_moved({ title: item.title });
			await reload();
		}
	}

	/** A copy next to the entry, named as KeePass names one. */
	async function duplicate(item: Item) {
		if (!mayEdit(item) || item.deleted) return;
		busy = true;
		const title = m.vault_copy_of({ title: item.title });
		if (item.source === 'personal' && key && item.entry.content) {
			const entry = await personalAsKdbx(key, item.entry);
			const id = entry && (await savePersonalKdbx(key, { ...entry, title }, item.folder));
			if (id) chosen = id;
		} else if (item.source === 'shared' && tree && mayReveal(item)) {
			const entry = await credentialAsKdbx(tree, item.credential);
			const made =
				entry && (await importInto(tree, item.collection, [{ ...entry, path: [], title }]));
			if (made && made.ids[0]) chosen = pickKey('credential', made.ids[0]);
		}
		busy = false;
		status = m.vault_duplicated({ title: item.title });
		await reload();
	}

	/** Into the recycle bin; from the bin, for good after asking. */
	async function remove(item: Item) {
		if (!mayEdit(item)) return;
		if (item.deleted) {
			show({ type: 'purge', item });
			return;
		}
		if (item.source === 'personal') {
			if (!key || !item.entry.content) return;
			const result = await saveEntry(key, item.id, {
				...item.entry.content,
				deleted: new Date().toISOString()
			});
			if (!result.ok) return fail(result.code);
		} else {
			const result = await deleteCredential(item.id);
			if (!result.ok) return fail(result.code);
		}
		if (chosen === item.key) chosen = null;
		status = m.vault_binned({ title: item.title });
		await reload();
	}

	async function restore(item: Item) {
		if (item.source === 'personal') {
			if (!key || !item.entry.content) return;
			const result = await saveEntry(key, item.id, { ...item.entry.content, deleted: null });
			if (!result.ok) return fail(result.code);
		} else {
			const result = await restoreCredential(item.id);
			if (!result.ok) return fail(result.code);
		}
		status = m.vault_restored({ title: item.title });
		await reload();
	}

	async function purge(item: Item) {
		busy = true;
		const done =
			item.source === 'personal'
				? await purgePersonal(item.entry)
				: (await deleteCredential(item.id, true)).ok;
		busy = false;
		if (!done) {
			error = errorMessage('internal');
			return;
		}
		close();
		if (chosen === item.key) chosen = null;
		await reload();
	}

	async function emptyBin(side: 'personal' | 'shared') {
		busy = true;
		for (const item of items.filter((i) => i.deleted && i.source === side && mayEdit(i))) {
			if (item.source === 'personal') await purgePersonal(item.entry);
			else await deleteCredential(item.id, true);
		}
		busy = false;
		close();
		chosen = null;
		await reload();
	}

	/** Drops an entry dragged from the table, or a personal folder, onto a folder of the tree. */
	function dropOn(event: DragEvent, target: Target) {
		event.preventDefault();
		dropOver = null;
		const folder = event.dataTransfer?.getData(FOLDER_DRAG_TYPE);
		if (folder) {
			if (target.side === 'shared' && takesFolder(target.id)) moveFolderToShared(folder, target.id);
			return;
		}
		const dragged = event.dataTransfer?.getData(DRAG_TYPE);
		const item = items.find((i) => i.key === dragged);
		if (item && !item.deleted) moveTo(item, target);
	}

	function dragOver(event: DragEvent, place: string, target: Target) {
		const types = event.dataTransfer?.types ?? [];
		const folder = types.includes(FOLDER_DRAG_TYPE);
		// A folder only into Shared, where a collection may be made (#218).
		if (folder ? target.side !== 'shared' || !takesFolder(target.id) : !types.includes(DRAG_TYPE))
			return;
		event.preventDefault();
		if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
		dropOver = place;
	}

	/**
	 * Whether a personal folder may go into `collection`, or into Shared itself
	 * with null: it becomes a collection there, which takes `manage` on the
	 * collection, or the right to make collections at the top (#218).
	 */
	function takesFolder(collection: string | null): boolean {
		if (collection === null) return tree?.may_create_top_level ?? false;
		const role = tree?.collections.find((c) => c.id === collection)?.role ?? null;
		return allows(role, 'manage');
	}

	function startFolderDrag(event: DragEvent, folder: string) {
		if (!event.dataTransfer) return;
		event.dataTransfer.effectAllowed = 'move';
		event.dataTransfer.setData(FOLDER_DRAG_TYPE, folder);
	}

	/**
	 * Moves a personal folder with everything in it into Shared (#218): it
	 * becomes a collection there, its folders collections below it and its
	 * entries credentials, as a single entry moves. The originals go only once
	 * everything arrived; what is in their recycle bin goes to the top.
	 */
	async function moveFolderToShared(folderId: string, into: string | null) {
		const folder = folders.find((f) => f.id === folderId);
		if (!key || !tree || stage !== 'open' || !folder || !takesFolder(into)) return;
		const opened = key;
		const inside = below(folders, folderId);
		const pathOf = (id: string): string[] => {
			const here = folders.find((f) => f.id === id);
			if (!here) return [];
			return id === folderId || !here.parent_id
				? [here.name]
				: [...pathOf(here.parent_id), here.name];
		};
		const moving = entries.filter(
			(e) =>
				e.content &&
				e.content.kind !== 'folder' &&
				!e.content.deleted &&
				e.content.parent &&
				inside.has(e.content.parent)
		);
		busy = true;
		status = null;
		const converted: KdbxEntry[] = [];
		for (const entry of moving) {
			const out = await personalAsKdbx(opened, entry);
			if (!out) {
				busy = false;
				status = m.vault_move_failed({ title: entry.content?.title ?? '' });
				return;
			}
			converted.push({ ...out, path: pathOf(entry.content?.parent ?? folderId) });
		}
		const groups = [...inside].map(pathOf);
		const result = await importInto(tree, into, converted, folder.name, groups);
		if (result.failed.length > 0) {
			busy = false;
			status = m.vault_folder_not_moved({ name: folder.name, names: result.failed.join(', ') });
			await reload();
			return;
		}
		for (const entry of moving) await purgePersonal(entry);
		for (const entry of entries) {
			if (entry.content?.deleted && entry.content.parent && inside.has(entry.content.parent)) {
				await saveEntry(opened, entry.id, { ...entry.content, parent: null });
			}
		}
		for (const id of inside) {
			const entry = entries.find((e) => e.id === id);
			if (entry) await purgePersonal(entry);
		}
		busy = false;
		status = m.vault_folder_moved({ name: folder.name });
		pick({ kind: 'personal', folder: folder.parent_id });
		await reload();
	}

	// ---- Context menu and shortcuts ----

	const ctrl = (letter: string) => `${m.key_ctrl()}+${letter}`;

	const menuItems = (item: Item): ContextItem[] =>
		item.deleted
			? [
					{ label: m.vault_restore(), disabled: !mayEdit(item), onselect: () => restore(item) },
					{
						label: m.vault_purge(),
						keys: m.key_delete(),
						danger: true,
						disabled: !mayEdit(item),
						onselect: () => remove(item)
					}
				]
			: [
					{
						label: m.vault_copy_username(),
						keys: ctrl('B'),
						onselect: () => copy(item, 'username')
					},
					{
						label: m.vault_copy_password(),
						keys: ctrl('C'),
						disabled: !mayReveal(item),
						onselect: () => copy(item, 'password')
					},
					{
						label: m.vault_copy_totp(),
						keys: ctrl('T'),
						disabled: !item.hasTotp || !mayReveal(item),
						onselect: () => copy(item, 'totp')
					},
					{
						label: m.vault_open_url(),
						keys: ctrl('U'),
						disabled: !/^https?:\/\//i.test(item.url),
						onselect: () => openUrl(item)
					},
					{
						label: m.vault_edit_entry(),
						keys: m.key_enter(),
						separated: true,
						disabled: !mayEdit(item),
						onselect: () => edit(item)
					},
					{
						label: m.vault_duplicate(),
						keys: ctrl('K'),
						disabled: !mayEdit(item) || !mayReveal(item),
						onselect: () => duplicate(item)
					},
					{
						label: m.vault_to_bin(),
						keys: m.key_delete(),
						separated: true,
						danger: true,
						disabled: !mayEdit(item),
						onselect: () => remove(item)
					}
				];

	function onkeydown(event: KeyboardEvent) {
		const target = event.target as HTMLElement | null;
		// A dialog of the entry pane, the file viewer say, is none of this page's.
		if (dialogOpen || menu || target?.closest('dialog')) return;
		const mod = event.ctrlKey || event.metaKey;
		const letter = event.key.toLowerCase();
		if (mod && letter === 'f') {
			event.preventDefault();
			search?.focus();
			return;
		}
		if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
		if (mod && letter === 'l') {
			event.preventDefault();
			lock();
			return;
		}
		if (mod && letter === 'i') {
			event.preventDefault();
			newEntry();
			return;
		}
		const item = current;
		if (!item) return;
		if (mod && letter === 'b') copy(item, 'username');
		// A text selection keeps the browser's own copy.
		else if (mod && letter === 'c' && !window.getSelection()?.toString()) copy(item, 'password');
		else if (mod && letter === 't') copy(item, 'totp');
		else if (mod && letter === 'u') openUrl(item);
		else if (mod && letter === 'k') duplicate(item);
		else if (!mod && event.key === 'Enter' && target?.getAttribute('role') === 'row') edit(item);
		else if (!mod && event.key === 'Delete') remove(item);
		else return;
		event.preventDefault();
	}

	// ---- Folders and collections ----

	async function addFolder(event: SubmitEvent) {
		event.preventDefault();
		if (!key) return;
		const parent = scope.kind === 'personal' ? scope.folder : null;
		const result = await saveEntry(key, null, {
			...EMPTY,
			kind: 'folder',
			parent,
			title: folderName.trim(),
			icon: FOLDER_ICON
		});
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		folderName = '';
		close();
		await loadPersonal();
		if (result.id) pick({ kind: 'personal', folder: result.id });
	}

	/** Only an empty folder goes, the recycle bin not counted. */
	const emptyFolder = (id: string) =>
		!entries.some((entry) => entry.content?.parent === id && !entry.content?.deleted);

	async function removeFolder(id: string) {
		const folder = entries.find((entry) => entry.id === id);
		const parent = folder?.content?.parent ?? null;
		// What lies in its bin goes back to the top, still in the bin.
		for (const entry of entries.filter((e) => e.content?.parent === id && e.content?.deleted)) {
			if (key && entry.content) await saveEntry(key, entry.id, { ...entry.content, parent: null });
		}
		if (folder) await purgePersonal(folder);
		pick({ kind: 'personal', folder: parent });
		await loadPersonal();
	}

	async function collectionSaved(id: string) {
		close();
		await loadShared();
		if (id) pick({ kind: 'collection', id });
	}

	async function removeCollection(collection: Collection) {
		const result = await deleteCollection(collection.id);
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		close();
		pick({ kind: 'all' });
		await loadShared();
	}

	/** Behind the gear of a collection: managing it, and KeePass files. */
	function collectionMenu(collection: Collection): MenuItem[] {
		return [
			...(allows(collection.role, 'manage')
				? [
						{
							label: m.vault_manage_collection(),
							icon: ShieldCheck,
							onselect: () => show({ type: 'collection', collection, parent: null })
						},
						{
							label: m.vault_new_subcollection(),
							icon: FolderPlus,
							onselect: () => show({ type: 'collection', collection: null, parent: collection.id })
						}
					]
				: []),
			...(allows(collection.role, 'edit')
				? [
						{
							label: m.kdbx_import(),
							icon: FileUp,
							onselect: () => show({ type: 'kdbx', mode: 'import', collection })
						}
					]
				: []),
			...(allows(collection.role, 'reveal')
				? [
						{
							label: m.kdbx_export(),
							icon: FileDown,
							onselect: () => show({ type: 'kdbx', mode: 'export', collection })
						}
					]
				: [])
		];
	}

	/** Behind the gear of the personal vault. */
	const personalMenu = $derived<MenuItem[]>(
		stage === 'open'
			? [
					{
						label: m.vault_new_folder(),
						icon: FolderPlus,
						onselect: () => show({ type: 'folder' })
					},
					...(scopeFolder && emptyFolder(scopeFolder.id)
						? [
								{
									label: m.vault_remove_folder(),
									icon: Trash2,
									danger: true,
									onselect: () => scopeFolder && removeFolder(scopeFolder.id)
								}
							]
						: []),
					// Without a pointer, what dragging does (#218).
					...(scopeFolder && sharedPlaces.length > 0
						? [
								{
									label: m.vault_move_folder(),
									icon: FolderInput,
									onselect: () =>
										scopeFolder &&
										show({ type: 'move-folder', folder: scopeFolder.id, name: scopeFolder.name })
								}
							]
						: []),
					{
						label: m.kdbx_import(),
						icon: FileUp,
						onselect: () => show({ type: 'kdbx', mode: 'import', collection: null })
					},
					{
						label: m.kdbx_export(),
						icon: FileDown,
						onselect: () => show({ type: 'kdbx', mode: 'export', collection: null })
					},
					{
						label: m.vault_unlocks_title(),
						icon: Fingerprint,
						onselect: () => show({ type: 'unlocks' })
					},
					{ label: m.vault_lock(), icon: Lock, onselect: lock }
				]
			: []
	);

	// ---- KeePass files ----

	async function importKdbx(imported: KdbxEntry[], file: string) {
		if (open?.type !== 'kdbx') return;
		const into = open.collection;
		busy = true;
		error = null;
		if ((into || open.shared) && tree) {
			// Into Shared itself, what lies at the file's top goes into a
			// collection named after the file (#216).
			const top = file.replace(/\.kdbx$/i, '').trim() || m.vault_shared();
			const result = await importInto(tree, into?.id ?? null, imported, top);
			kdbxResult = m.kdbx_imported({ count: result.created });
			if (result.failed.length > 0)
				error = m.kdbx_not_imported({ names: result.failed.join(', ') });
			await loadShared();
		} else if (key) {
			kdbxResult = m.kdbx_imported({ count: await importPersonal(key, imported) });
			await loadPersonal();
		}
		busy = false;
	}

	/** Takes a KeePass file's entries into the personal folder shown (#99). */
	async function importPersonal(opened: CryptoKey, imported: KdbxEntry[]): Promise<number> {
		const top = scope.kind === 'personal' ? scope.folder : null;
		const made: Record<string, string | null> = { '': top };
		const folderOf = async (path: string[]): Promise<string | null> => {
			const at = path.join('\n');
			if (at in made) return made[at];
			const parent = await folderOf(path.slice(0, -1));
			const saved = await saveEntry(opened, null, {
				...EMPTY,
				kind: 'folder',
				parent,
				title: path[path.length - 1],
				icon: FOLDER_ICON
			});
			const id = saved.ok ? saved.id : parent;
			made[at] = id;
			return id;
		};
		let created = 0;
		for (const item of imported) {
			if (await savePersonalKdbx(opened, item, await folderOf(item.path))) created += 1;
		}
		return created;
	}

	async function exportKdbx(password: string) {
		if (open?.type !== 'kdbx') return;
		const from = open.collection;
		busy = true;
		error = null;
		if (from && tree) {
			const result = await exportCollection(tree, from.id);
			if (!result.ok) {
				busy = false;
				error =
					'missing' in result
						? m.kdbx_missing_reveal({ name: result.missing })
						: m.kdbx_export_failed({ name: result.failed });
				return;
			}
			hand(new Blob([await writeKdbx(result.entries, password, from.name)]), `${from.name}.kdbx`);
			kdbxResult = m.kdbx_exported({ count: result.entries.length });
		} else if (key) {
			const opened = key;
			const out = await asKdbx(entries, (ref) => readFile(opened, ref));
			hand(new Blob([await writeKdbx(out, password, m.vault_title())]), 'remotehub-vault.kdbx');
			kdbxResult = m.kdbx_exported({ count: out.length });
		}
		busy = false;
	}

	/** Opens a personal file in the browser and hands it over as a download. */
	async function download(ref: FileRef) {
		if (!key) return;
		const blob = await readFile(key, ref);
		if (!blob) {
			error = errorMessage('not_found');
			return;
		}
		hand(blob, ref.name);
	}

	function hand(blob: Blob, name: string) {
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.href = url;
		link.download = name;
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
	}

	// ---- Ways to unlock ----

	async function newPasskey() {
		if (!key || !session.user) return;
		error = null;
		try {
			const result = await addPasskey(key, session.user.username, passkeyLabel.trim());
			if (!result.ok) {
				error =
					result.code === 'prf_unsupported' ? m.vault_prf_unsupported() : errorMessage(result.code);
				return;
			}
		} catch {
			error = m.vault_prf_unsupported();
			return;
		}
		passkeyLabel = '';
		await loadPersonal();
	}

	async function changePassphrase(event: SubmitEvent) {
		event.preventDefault();
		error = passphraseProblem();
		if (error || !key) return;
		busy = true;
		const result = await savePassphrase(key, passphrase);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		passphrase = passphraseAgain = '';
		close();
		await loadPersonal();
	}

	async function newRecovery() {
		if (!key) return;
		const result = await renewRecovery(key);
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		close();
		shownRecovery = result.recovery;
		stage = 'recovery';
	}

	async function dropUnlock(id: string) {
		const result = await removeUnlock(id);
		error = result.ok ? null : errorMessage(result.code);
		await loadPersonal();
	}

	async function startOver() {
		const result = await resetVault();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		close();
		key = unlocked.key = null;
		entries = [];
		stage = 'setup';
		await loadPersonal();
	}

	function sendRequest(role: Role, minutes: number, reason: string) {
		if (open?.type !== 'request') return;
		const id = open.credential.id;
		createRequest({ kind: 'credential', id }, role, minutes, reason).then((result) => {
			if (!result.ok) {
				error = problemMessage(result);
				return;
			}
			close();
			status = m.request_sent();
		});
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
	const tool =
		'inline-flex h-9 items-center gap-1.5 rounded-lg border border-line-strong bg-surface px-2.5 text-sm hover:bg-surface-2 disabled:opacity-40';
	const iconTool =
		'inline-flex size-9 items-center justify-center rounded-lg border border-line-strong bg-surface hover:bg-surface-2 disabled:opacity-40';
	const navItem =
		'flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-surface-2 aria-[current=true]:bg-accent/15 aria-[current=true]:font-semibold data-[drop=true]:ring-2 data-[drop=true]:ring-accent';
</script>

<svelte:window {onkeydown} />

{#snippet passphraseFields()}
	<label class={label} for="vault-passphrase">{m.field_passphrase()}</label>
	<input
		id="vault-passphrase"
		type="password"
		class={field}
		required
		autocomplete="new-password"
		bind:value={passphrase}
	/>
	<label class={label} for="vault-passphrase-again">{m.field_passphrase_again()}</label>
	<input
		id="vault-passphrase-again"
		type="password"
		class={field}
		required
		autocomplete="new-password"
		bind:value={passphraseAgain}
	/>
{/snippet}

{#snippet problem()}
	{#if error}
		<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="shrink-0 text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
{/snippet}

{#snippet node(
	target: Scope,
	text: string,
	depth: number,
	glyph: typeof Lock | null,
	drop: Target | null
)}
	{@const Glyph = glyph}
	{@const place = drop ? placeOf(drop) : null}
	<!-- A personal folder goes into Shared by dragging (#218). -->
	{@const folder =
		stage === 'open' && target.kind === 'personal' && target.folder ? target.folder : null}
	<button
		type="button"
		class={navItem}
		style:padding-left="{0.5 + depth * 1}rem"
		aria-current={!searching && sameScope(scope, target)}
		data-drop={place !== null && dropOver === place}
		draggable={folder !== null}
		onclick={() => pick(target)}
		ondragstart={(event) => folder && startFolderDrag(event, folder)}
		ondragover={(event) => drop && place && dragOver(event, place, drop)}
		ondragleave={() => (dropOver = null)}
		ondrop={(event) => drop && dropOn(event, drop)}
	>
		{#if Glyph}<Glyph size={15} class="shrink-0 text-ink-3" aria-hidden="true" />{/if}
		<span class="flex-1 truncate">{text}</span>
		{#if place !== null && dropOver === place}
			<!-- Where it goes, not by colour alone. -->
			<FolderInput size={15} class="shrink-0 text-accent" aria-hidden="true" />
		{:else}
			<span class="text-xs text-ink-3 tabular-nums">{count(target) || ''}</span>
		{/if}
	</button>
{/snippet}

<!-- The personal vault's own stages, where its entries would be. -->
{#snippet personalStage()}
	{#if stage === 'setup'}
		<form class="m-4 max-w-lg rounded-card border border-line bg-surface p-5" onsubmit={create}>
			<h2 class="text-lg font-semibold">{m.vault_setup_title()}</h2>
			<p class="mt-1 text-sm text-ink-2">{m.vault_setup_hint()}</p>
			{@render passphraseFields()}
			<button type="submit" class="{primary} mt-5" disabled={busy}>{m.vault_set_up()}</button>
			{@render problem()}
		</form>
	{:else if stage === 'locked'}
		<form class="m-4 max-w-lg rounded-card border border-line bg-surface p-5" onsubmit={unlock}>
			<h2 class="text-lg font-semibold">{m.vault_unlock_title()}</h2>
			<p class="mt-1 text-sm text-ink-2">{m.vault_locked_hint()}</p>
			{#if useRecovery}
				<label class={label} for="vault-recovery">{m.field_recovery_key()}</label>
				<input
					id="vault-recovery"
					class="{field} font-mono"
					required
					autocomplete="off"
					spellcheck="false"
					bind:value={recoveryText}
				/>
			{:else}
				<label class={label} for="vault-unlock-passphrase">{m.field_passphrase()}</label>
				<input
					id="vault-unlock-passphrase"
					type="password"
					class={field}
					required
					autocomplete="current-password"
					bind:value={passphrase}
				/>
			{/if}
			<div class="mt-5 flex flex-wrap gap-2">
				<button type="submit" class={primary} disabled={busy}>{m.vault_unlock()}</button>
				{#if passkeysAvailable() && vault?.unlocks.some((u) => u.kind === 'passkey')}
					<button type="button" class={button} onclick={unlockPasskey}>
						<Fingerprint size={16} aria-hidden="true" />
						{m.vault_unlock_passkey()}
					</button>
				{/if}
				<button type="button" class={button} onclick={() => (useRecovery = !useRecovery)}>
					{useRecovery ? m.vault_use_passphrase() : m.vault_use_recovery()}
				</button>
			</div>
			<button
				type="button"
				class="mt-4 text-xs text-ink-3 underline"
				onclick={() => show({ type: 'reset' })}
			>
				{m.vault_reset()}
			</button>
			{@render problem()}
		</form>
	{/if}
{/snippet}

<div class="flex min-h-0 flex-1 flex-col">
	<div
		role="toolbar"
		aria-label={m.vault_toolbar()}
		class="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-line bg-surface px-3 py-2"
	>
		<h1 class="mr-2 px-1 eyebrow">{m.nav_vault()}</h1>
		<button
			type="button"
			class={primary}
			disabled={!newPlace() || stage === 'recovery'}
			title={`${m.vault_new_entry()} (${ctrl('I')})`}
			onclick={newEntry}
		>
			<Plus size={16} aria-hidden="true" />
			{m.vault_new_entry()}
		</button>
		<span class="mx-1 h-6 w-px bg-line" aria-hidden="true"></span>
		<button
			type="button"
			class={tool}
			disabled={!current || current.deleted}
			title={`${m.vault_copy_username()} (${ctrl('B')})`}
			onclick={() => current && copy(current, 'username')}
		>
			<User size={15} aria-hidden="true" />
			<span class="hidden xl:inline">{m.vault_copy_username()}</span>
			<span class="sr-only xl:hidden">{m.vault_copy_username()}</span>
		</button>
		<button
			type="button"
			class={tool}
			disabled={!current || current.deleted || !mayReveal(current)}
			title={`${m.vault_copy_password()} (${ctrl('C')})`}
			onclick={() => current && copy(current, 'password')}
		>
			<KeyRound size={15} aria-hidden="true" />
			<span class="hidden xl:inline">{m.vault_copy_password()}</span>
			<span class="sr-only xl:hidden">{m.vault_copy_password()}</span>
		</button>
		<button
			type="button"
			class={tool}
			disabled={!current || !current.hasTotp || current.deleted || !mayReveal(current)}
			title={`${m.vault_copy_totp()} (${ctrl('T')})`}
			onclick={() => current && copy(current, 'totp')}
		>
			<Clock size={15} aria-hidden="true" />
			<span class="hidden xl:inline">{m.vault_copy_totp()}</span>
			<span class="sr-only xl:hidden">{m.vault_copy_totp()}</span>
		</button>
		<button
			type="button"
			class={tool}
			disabled={!current || !/^https?:\/\//i.test(current.url)}
			title={`${m.vault_open_url()} (${ctrl('U')})`}
			onclick={() => current && openUrl(current)}
		>
			<ExternalLink size={15} aria-hidden="true" />
			<span class="hidden xl:inline">{m.vault_open_url()}</span>
			<span class="sr-only xl:hidden">{m.vault_open_url()}</span>
		</button>
		<span class="mx-1 h-6 w-px bg-line" aria-hidden="true"></span>
		<button
			type="button"
			class={iconTool}
			disabled={!current || current.deleted || !mayEdit(current)}
			title={`${m.vault_edit_entry()} (${m.key_enter()})`}
			onclick={() => current && edit(current)}
		>
			<Pencil size={15} aria-hidden="true" />
			<span class="sr-only">{m.vault_edit_entry()}</span>
		</button>
		<button
			type="button"
			class="{iconTool} text-critical"
			disabled={!current || !mayEdit(current)}
			title={`${current?.deleted ? m.vault_purge() : m.vault_to_bin()} (${m.key_delete()})`}
			onclick={() => current && remove(current)}
		>
			<Trash2 size={15} aria-hidden="true" />
			<span class="sr-only">{current?.deleted ? m.vault_purge() : m.vault_to_bin()}</span>
		</button>
		<label class="relative ml-auto block w-full max-w-xs">
			<span class="sr-only">{m.vault_search()}</span>
			<Search size={15} class="absolute top-2.5 left-2.5 text-ink-3" aria-hidden="true" />
			<input
				bind:this={search}
				type="search"
				class="h-9 w-full rounded-lg border border-line-strong bg-page pr-3 pl-8 text-sm"
				placeholder={`${m.vault_search()} (${ctrl('F')})`}
				bind:value={query}
			/>
		</label>
		{#if stage === 'open'}
			<button
				type="button"
				class={iconTool}
				title={`${m.vault_lock()} (${ctrl('L')})`}
				onclick={lock}
			>
				<Lock size={15} aria-hidden="true" />
				<span class="sr-only">{m.vault_lock()}</span>
			</button>
		{/if}
	</div>

	<div class="flex min-h-0 flex-1 flex-col lg:flex-row">
		<nav
			aria-label={m.vault_folders_title()}
			class="relative flex shrink-0 flex-col gap-4 border-b border-line bg-sunken px-2 py-3 lg:w-64 lg:overflow-y-auto lg:border-r lg:border-b-0"
		>
			<div class="flex flex-col gap-0.5">
				{@render node({ kind: 'all' }, m.vault_all(), 0, Layers, null)}
				{@render node({ kind: 'recent' }, m.vault_recent(), 0, Clock, null)}
				{@render node({ kind: 'expiring' }, m.vault_expiring(), 0, TriangleAlert, null)}
			</div>

			<section class="flex flex-col gap-0.5" aria-labelledby="vault-personal">
				<div class="flex items-center gap-2 px-2 pb-1">
					<h2 id="vault-personal" class="eyebrow">{m.vault_personal()}</h2>
					{#if stage === 'locked'}
						<span class="inline-flex items-center gap-1 text-xs text-warning">
							<Lock size={12} aria-hidden="true" />
							{m.vault_locked()}
						</span>
					{/if}
					<span class="ml-auto">
						<SettingsMenu label={m.vault_personal_settings()} items={personalMenu} />
					</span>
				</div>
				{@render node(
					{ kind: 'personal', folder: null },
					m.vault_personal_all(),
					0,
					stage === 'open' ? LockOpen : Lock,
					stage === 'open' ? { side: 'personal', folder: null } : null
				)}
				{#each personalOutline as folder (folder.id)}
					{@render node(
						{ kind: 'personal', folder: folder.id },
						folder.name,
						folder.depth + 1,
						FolderIcon,
						{ side: 'personal', folder: folder.id }
					)}
				{/each}
				{#if stage === 'open'}
					{@render node({ kind: 'bin', side: 'personal' }, m.vault_bin(), 1, Trash2, null)}
				{/if}
			</section>

			<section class="flex flex-col gap-0.5" aria-labelledby="vault-shared">
				<!-- A personal folder dropped here becomes a collection at the top (#218). -->
				<div
					role="presentation"
					class="flex items-center gap-2 rounded-md px-2 pb-1 data-[drop=true]:ring-2 data-[drop=true]:ring-accent"
					data-drop={dropOver === SHARED_TOP}
					ondragover={(event) => {
						if (!event.dataTransfer?.types.includes(FOLDER_DRAG_TYPE) || !takesFolder(null)) return;
						event.preventDefault();
						dropOver = SHARED_TOP;
					}}
					ondragleave={() => (dropOver = null)}
					ondrop={(event) => {
						event.preventDefault();
						dropOver = null;
						const folder = event.dataTransfer?.getData(FOLDER_DRAG_TYPE);
						if (folder && takesFolder(null)) moveFolderToShared(folder, null);
					}}
				>
					<h2 id="vault-shared" class="eyebrow">{m.vault_shared()}</h2>
					{#if dropOver === SHARED_TOP}
						<FolderInput size={15} class="shrink-0 text-accent" aria-hidden="true" />
					{/if}
					{#if tree?.may_create_top_level}
						<button
							type="button"
							class="ml-auto flex size-7 items-center justify-center rounded-lg text-ink-2 hover:bg-surface-2 hover:text-ink"
							title={m.vault_new_collection()}
							onclick={() => show({ type: 'collection', collection: null, parent: null })}
						>
							<Plus size={15} aria-hidden="true" />
							<span class="sr-only">{m.vault_new_collection()}</span>
						</button>
						<SettingsMenu
							label={m.vault_shared_settings()}
							items={[
								{
									label: m.kdbx_import(),
									icon: FileUp,
									onselect: () =>
										show({ type: 'kdbx', mode: 'import', collection: null, shared: true })
								}
							]}
						/>
					{/if}
				</div>
				{#each collectionOutline as folder (folder.id)}
					{@const collection = tree?.collections.find((c) => c.id === folder.id)}
					<div class="group flex items-center">
						{@render node(
							{ kind: 'collection', id: folder.id },
							folder.name,
							folder.depth,
							Users,
							allows(collection?.role ?? null, 'edit') ? { side: 'shared', id: folder.id } : null
						)}
						{#if collection}
							<!-- Only where the pointer or focus is, and on the folder shown. -->
							<span
								class="shrink-0 opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 data-[shown=true]:opacity-100"
								data-shown={scope.kind === 'collection' && scope.id === folder.id}
							>
								<SettingsMenu
									label={m.vault_collection_settings({ name: folder.name })}
									items={collectionMenu(collection)}
								/>
							</span>
						{/if}
					</div>
				{:else}
					<p class="px-2.5 text-sm text-ink-3">{m.vault_no_collections()}</p>
				{/each}
				{#if (tree?.collections.length ?? 0) > 0}
					{@render node({ kind: 'bin', side: 'shared' }, m.vault_bin(), 0, Trash2, null)}
				{/if}
			</section>
		</nav>

		{#if stage === 'recovery'}
			<section class="relative flex-1 p-6 lg:overflow-y-auto">
				<div class="max-w-md rounded-card border border-line bg-surface p-6">
					<h2 class="text-lg font-semibold">{m.vault_recovery_title()}</h2>
					{#if mustRenew}
						<p class="mt-1 text-sm" role="status">{m.vault_recovered_hint()}</p>
					{/if}
					<p class="mt-1 text-sm text-ink-2">{m.vault_recovery_hint()}</p>
					<p
						class="mt-4 rounded-lg bg-surface-2 p-3 font-mono text-lg break-all"
						data-testid="recovery-key"
					>
						{shownRecovery}
					</p>
					<button type="button" class="{primary} mt-5" onclick={afterRecovery}>
						{m.vault_recovery_saved()}
					</button>
				</div>
			</section>
		{:else}
			<section
				class="flex min-h-[24rem] min-w-0 flex-1 flex-col lg:min-h-0"
				aria-labelledby="vault-scope"
			>
				<div class="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2">
					<h2 id="vault-scope" class="truncate font-semibold">{scopeTitle}</h2>
					{#if scope.kind === 'bin' && !searching && count(scope) > 0}
						<button
							type="button"
							class="{button} ml-auto text-critical"
							onclick={() =>
								show({ type: 'empty', side: scope.kind === 'bin' ? scope.side : 'shared' })}
						>
							<Trash2 size={14} aria-hidden="true" />
							{m.vault_empty_bin()}
						</button>
					{/if}
				</div>
				<!-- The setup and unlock forms show their own. -->
				{#if !dialogOpen && !staging}{@render problem()}{/if}
				{#if staging}
					<div class="relative flex-1 lg:overflow-y-auto">{@render personalStage()}</div>
				{:else}
					{#if (stage === 'locked' || stage === 'setup') && !searching && scope.kind === 'all'}
						<button
							type="button"
							class="mx-4 mt-3 flex items-center gap-2 rounded-lg border border-line bg-surface px-3 py-2 text-left text-sm text-ink-2 hover:bg-surface-2"
							onclick={() => pick({ kind: 'personal', folder: null })}
						>
							<Lock size={15} class="shrink-0 text-warning" aria-hidden="true" />
							{stage === 'locked' ? m.vault_locked_notice() : m.vault_setup_notice()}
						</button>
					{/if}
					<EntryTable
						items={shown}
						{sort}
						{chosen}
						label={scopeTitle}
						onsort={(next) => (sort = next)}
						onchoose={choose}
						onedit={edit}
						oncopy={copy}
						onopenurl={openUrl}
						oncontext={(item, at) => (menu = { item, at })}
					/>
					{#if scope.kind === 'personal' && scope.folder === null && !searching && unreadable.length > 0}
						<ul class="flex shrink-0 flex-col gap-1 border-t border-line p-2">
							{#each unreadable as entry (entry.id)}
								<li class="flex items-center gap-3 px-3 py-1.5 text-sm">
									<CircleAlert size={16} class="shrink-0 text-critical" aria-hidden="true" />
									<span class="flex-1 text-ink-2">{m.vault_unreadable()}</span>
									<button
										type="button"
										class={button}
										onclick={() => purgePersonal(entry).then(loadPersonal)}
									>
										{m.catalog_delete()}
									</button>
								</li>
							{/each}
						</ul>
					{/if}
				{/if}
				{#if current && !staging}
					<EntryPane
						item={current}
						oncopy={copy}
						onedit={() => edit(current)}
						onrestore={() => restore(current)}
						onrequest={() =>
							current.source === 'shared' &&
							show({
								type: 'request',
								credential: current.credential,
								role: current.credential.role
							})}
						onread={(ref) => (key ? readFile(key, ref) : Promise.resolve(null))}
						ondownload={download}
					/>
				{/if}
				<footer
					class="flex shrink-0 items-center gap-4 border-t border-line bg-surface px-4 py-1 text-xs text-ink-2"
				>
					<span class="tabular-nums">{m.vault_count({ count: shown.length })}</span>
					<span role="status" class="truncate">
						{status ?? ''}
						{#if clearing}
							{m.vault_clipboard_clears({ seconds: clearing.left })}
						{/if}
					</span>
					<span class="ml-auto hidden truncate text-ink-3 xl:inline"
						>{m.vault_shortcuts_hint()}</span
					>
				</footer>
			</section>
		{/if}
	</div>
</div>

{#if menu}
	<ContextMenu
		label={m.vault_entry_menu({ title: menu.item.title })}
		items={menuItems(menu.item)}
		at={menu.at}
		onclose={() => (menu = null)}
	/>
{/if}

<Dialog
	bind:open={dialogOpen}
	title={dialogTitle}
	wide={open?.type === 'entry' || (open?.type === 'collection' && !!open.collection)}
>
	{#if dialogOpen && open?.type === 'entry'}
		{@const item = open.item}
		<EntryEditor
			draft={open.draft}
			shared={item ? item.source === 'shared' : open.draft.place.startsWith('s:')}
			isNew={!item}
			{places}
			files={item?.source === 'personal'
				? (item.entry.content?.attachments ?? [])
				: item?.source === 'shared'
					? item.credential.attachments
					: []}
			hasTotp={item?.source === 'shared' && item.hasTotp}
			history={item?.source === 'personal' ? (item.entry.content?.history ?? []) : []}
			credentialId={item?.source === 'shared' ? item.id : null}
			mayReveal={item ? mayReveal(item) : false}
			{busy}
			{error}
			onsubmit={saveDraft}
			oncancel={close}
		/>
	{:else if dialogOpen && open?.type === 'folder'}
		<form onsubmit={addFolder}>
			<label class="block text-sm font-medium" for="vault-folder-name">{m.field_name()}</label>
			<input
				id="vault-folder-name"
				class={field}
				required
				maxlength="200"
				bind:value={folderName}
			/>
			<button type="submit" class="{primary} mt-5">{m.action_create()}</button>
			{@render problem()}
		</form>
	{:else if dialogOpen && open?.type === 'collection' && tree}
		{@const target = open}
		<CollectionForm
			{tree}
			collection={target.collection}
			parent={target.parent}
			onsaved={collectionSaved}
			oncancel={close}
		/>
		{#if target.collection}
			{@const collection = target.collection}
			<div class="mt-6 border-t border-line pt-5">
				<h3 class="font-semibold">{m.catalog_permissions()}</h3>
				<p class="mt-1 text-sm text-ink-2">{m.vault_collection_grants_hint()}</p>
				<div class="mt-3">
					<Grants kind="collection" id={collection.id} />
				</div>
			</div>
			<div class="mt-6 flex border-t border-line pt-4">
				<button
					type="button"
					class="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm text-critical hover:bg-surface-2"
					onclick={() => removeCollection(collection)}
				>
					<Trash2 size={15} aria-hidden="true" />
					{m.vault_delete_collection()}
				</button>
			</div>
			{@render problem()}
		{/if}
	{:else if dialogOpen && open?.type === 'purge'}
		{@const item = open.item}
		<p class="text-sm">{m.vault_purge_confirm({ title: item.title })}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={close}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
				disabled={busy}
				onclick={() => purge(item)}
			>
				{m.vault_purge()}
			</button>
		</div>
		{@render problem()}
	{:else if dialogOpen && open?.type === 'empty'}
		{@const side = open.side}
		<p class="text-sm">{m.vault_empty_bin_confirm({ count: count({ kind: 'bin', side }) })}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={close}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
				disabled={busy}
				onclick={() => emptyBin(side)}
			>
				{m.vault_empty_bin()}
			</button>
		</div>
	{:else if dialogOpen && open?.type === 'request'}
		<RequestForm held={open.role} kind="credential" onsubmit={sendRequest} oncancel={close} />
		{@render problem()}
	{:else if dialogOpen && open?.type === 'move-folder'}
		{@const moving = open}
		<form
			onsubmit={async (event) => {
				event.preventDefault();
				if (!moveInto) return;
				close();
				await moveFolderToShared(moving.folder, moveInto === 'top' ? null : moveInto);
			}}
		>
			<p class="text-sm text-ink-2">{m.vault_move_folder_hint()}</p>
			<label class={label} for="move-folder-into">{m.vault_move_folder_into()}</label>
			<select id="move-folder-into" class={field} required bind:value={moveInto}>
				<option value="" disabled>{m.vault_move_folder_choose()}</option>
				{#each sharedPlaces as place (place.id)}
					<option value={place.id}>{place.path || m.vault_move_folder_top()}</option>
				{/each}
			</select>
			<div class="mt-5 flex justify-end gap-2">
				<button
					type="button"
					class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
					onclick={close}
				>
					{m.action_cancel()}
				</button>
				<button type="submit" class={primary} disabled={busy}>{m.vault_move_folder_submit()}</button
				>
			</div>
		</form>
	{:else if dialogOpen && open?.type === 'kdbx'}
		<KdbxForm mode={open.mode} {busy} onimport={importKdbx} onexport={exportKdbx} />
		{#if kdbxResult}
			<p class="mt-3 text-sm" role="status">{kdbxResult}</p>
		{/if}
		{@render problem()}
	{:else if dialogOpen && open?.type === 'unlocks'}
		<ul class="divide-y divide-line rounded-card border border-line bg-surface">
			{#each vault?.unlocks ?? [] as way (way.id)}
				<li class="flex items-center gap-3 px-4 py-2 text-sm">
					<span>{KIND_LABELS[way.kind]()}{way.label ? ` · ${way.label}` : ''}</span>
					{#if way.kind === 'passkey'}
						<button
							type="button"
							class="ml-auto rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
							onclick={() => dropUnlock(way.id)}
						>
							{m.vault_remove()}
						</button>
					{/if}
				</li>
			{/each}
		</ul>
		{#if passkeysAvailable()}
			<div class="mt-4 flex flex-wrap items-end gap-2">
				<label class="text-sm">
					<span class="block font-medium">{m.vault_passkey_label()}</span>
					<input
						class="mt-1 rounded-lg border border-line bg-page px-3 py-1.5"
						bind:value={passkeyLabel}
					/>
				</label>
				<button type="button" class={button} onclick={newPasskey}>
					<Fingerprint size={16} aria-hidden="true" />
					{m.vault_add_passkey()}
				</button>
			</div>
		{/if}
		<div class="mt-4 flex flex-wrap gap-2">
			<button type="button" class={button} onclick={() => show({ type: 'passphrase' })}>
				<Pencil size={15} aria-hidden="true" />
				{m.vault_change_passphrase()}
			</button>
			<button type="button" class={button} onclick={newRecovery}>{m.vault_new_recovery()}</button>
		</div>
		{@render problem()}
	{:else if dialogOpen && open?.type === 'passphrase'}
		<form onsubmit={changePassphrase}>
			{@render passphraseFields()}
			<button type="submit" class="{primary} mt-5" disabled={busy}>{m.action_save()}</button>
			{@render problem()}
		</form>
	{:else if dialogOpen && open?.type === 'reset'}
		<p class="text-sm">{m.vault_reset_confirm()}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={close}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={startOver}
			>
				{m.vault_reset()}
			</button>
		</div>
	{/if}
</Dialog>
