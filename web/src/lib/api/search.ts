/** What the user picked after searching the devices (crates/server/src/api/search.rs). */
import type { Pick } from '$lib/search/rank';
import { api } from './client';

export const loadPicks = () => api<Pick[]>('GET', '/api/search/picks');
export const savePick = (key: string, query: string) =>
	api('POST', '/api/search/picks', { key, query });
