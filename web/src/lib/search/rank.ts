/**
 * Search that finds parts of words and learns from what people pick (#81).
 *
 * Matching: every word of the query must occur in one of an item's fields,
 * ignoring case and accents. A word that is a whole field counts most, then
 * one the field starts with, one a word in it starts with, one anywhere in
 * it; each weighted by how telling the field is (a name more than a notes
 * field).
 *
 * Learning: every pick is kept as (item, query, count, last time). An item
 * earns a bonus for the current query from picks with the same query, with
 * a query the current one is being typed towards, or one it continues; each
 * counts less the older it is. The bonus only reorders what matches, it
 * never brings in what does not.
 */

/** One text of an item, and how much a match in it counts (0..1). */
export interface Field {
	text: string;
	weight: number;
}

export interface Searchable<T> {
	/** Stable key the picks refer to, e.g. `device:<id>`. */
	key: string;
	/** Tie-breaker, shown first in the results. */
	name: string;
	fields: Field[];
	item: T;
}

/** What someone picked after searching for `query` (normalized). */
export interface Pick {
	key: string;
	query: string;
	count: number;
	/** Milliseconds since the epoch. */
	last: number;
}

/** Picks kept at most; the least useful go first. */
export const MAX_PICKS = 300;
/** A pick counts half as much after this many days. */
const HALF_LIFE_DAYS = 14;
const DAY = 24 * 60 * 60 * 1000;

/** Lower case without accents: "Büro" and "buro" match. */
export function normalize(text: string): string {
	return text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

export function words(query: string): string[] {
	return normalize(query).split(/\s+/).filter(Boolean);
}

/** A query as picks keep it: normalized words, at most 100 characters. */
export const queryKey = (query: string) => words(query).join(' ').slice(0, 100);

/** How well `word` matches `text` (both normalized): 0 not at all, 100 exactly. */
export function wordScore(text: string, word: string): number {
	if (text === word) return 100;
	if (text.startsWith(word)) return 80;
	const at = text.indexOf(word);
	if (at < 0) return 0;
	// The start of a word inside the text: after anything but a letter or digit.
	for (let index = at; index >= 0; index = text.indexOf(word, index + 1)) {
		if (!/[\p{L}\p{N}]/u.test(text[index - 1])) return 65;
	}
	return 45;
}

/** The average best match of each word, or null if one word matches nowhere. */
export function matchScore(fields: Field[], query: string[]): number | null {
	if (query.length === 0) return null;
	const texts = fields.map((field) => ({ text: normalize(field.text), weight: field.weight }));
	let total = 0;
	for (const word of query) {
		const best = Math.max(0, ...texts.map((f) => wordScore(f.text, word) * f.weight));
		if (best === 0) return null;
		total += best;
	}
	return total / query.length;
}

const decay = (pick: Pick, now: number) => 0.5 ** ((now - pick.last) / DAY / HALF_LIFE_DAYS);

/** How much a pick made for `earlier` tells about the query `now` typed. */
function related(earlier: string, current: string): number {
	if (earlier === current) return 1;
	// Still typing towards a query that was picked from before.
	if (earlier.startsWith(current)) return 0.8;
	// Typed on past a query that was picked from before.
	if (current.startsWith(earlier)) return 0.5;
	return 0;
}

/** The learned bonus of every key for `query` (normalized). */
export function learned(picks: Pick[], query: string, now: number): Map<string, number> {
	const bonus = new Map<string, number>();
	for (const pick of picks) {
		const weight = related(pick.query, query);
		if (weight > 0) {
			bonus.set(pick.key, (bonus.get(pick.key) ?? 0) + pick.count * weight * decay(pick, now));
		}
	}
	return bonus;
}

/** What matches `query`, best first. */
export function rank<T>(
	items: Searchable<T>[],
	query: string,
	picks: Pick[],
	now: number,
	locale?: string
): T[] {
	const queryWords = words(query);
	const bonus = learned(picks, queryKey(query), now);
	return items
		.flatMap((entry) => {
			const match = matchScore(entry.fields, queryWords);
			if (match === null) return [];
			// Picks lift a result by up to about one match class per doubling.
			const score = match + 20 * Math.log2(1 + (bonus.get(entry.key) ?? 0));
			return [{ entry, score }];
		})
		.sort(
			(a, b) =>
				b.score - a.score ||
				a.entry.name.localeCompare(b.entry.name, locale, { sensitivity: 'base' })
		)
		.map(({ entry }) => entry.item);
}

/** The keys picked most, with or without a query, for a list of favourites. */
export function frequent(picks: Pick[], now: number, count: number): string[] {
	const total = new Map<string, number>();
	for (const pick of picks) {
		total.set(pick.key, (total.get(pick.key) ?? 0) + pick.count * decay(pick, now));
	}
	return [...total.entries()]
		.sort((a, b) => b[1] - a[1])
		.slice(0, count)
		.map(([key]) => key);
}

/** The picks with one more pick of `key` for `query`, trimmed to MAX_PICKS. */
export function remember(picks: Pick[], key: string, query: string, now: number): Pick[] {
	const normalized = queryKey(query);
	const known = picks.find((pick) => pick.key === key && pick.query === normalized);
	const next = known
		? picks.map((pick) => (pick === known ? { ...pick, count: pick.count + 1, last: now } : pick))
		: [...picks, { key, query: normalized, count: 1, last: now }];
	if (next.length <= MAX_PICKS) return next;
	// Keep what still counts most.
	return [...next]
		.sort((a, b) => b.count * decay(b, now) - a.count * decay(a, now))
		.slice(0, MAX_PICKS);
}
