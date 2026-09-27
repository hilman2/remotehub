/** The password generator's settings (crates/server/src/api/generator.rs, #194). */
import type { GeneratorSettings } from '$lib/vault/generate';
import { api } from './client';

export interface GeneratorDefaults {
	/** Built in until an administrator sets it. */
	organisation: GeneratorSettings;
	/** The user's own; null: the organisation's applies. */
	own: GeneratorSettings | null;
}

export const loadGeneratorDefaults = () => api<GeneratorDefaults>('GET', '/api/generator');
export const saveOwnDefault = (settings: GeneratorSettings) =>
	api('PUT', '/api/generator/own', settings);
export const removeOwnDefault = () => api('DELETE', '/api/generator/own');
export const saveOrganisationDefault = (settings: GeneratorSettings) =>
	api('PUT', '/api/settings/generator', settings);
