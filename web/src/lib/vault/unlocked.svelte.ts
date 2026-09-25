/**
 * The key of the personal vault while it is unlocked (#92): in memory only,
 * for as long as the page is loaded, so connections can use its entries.
 * Locking, signing out or a reload forgets it.
 */
export const unlocked = $state<{ key: CryptoKey | null }>({ key: null });
