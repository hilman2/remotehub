import { describe, expect, it } from 'vitest';
import { requestableRoles } from './requests';

describe('access requests', () => {
	it('offer only more than one holds, and only connect and reveal', () => {
		expect(requestableRoles('list')).toEqual(['connect', 'reveal']);
		expect(requestableRoles('connect')).toEqual(['reveal']);
		expect(requestableRoles('reveal')).toEqual([]);
		expect(requestableRoles('manage')).toEqual([]);
		// A folder seen only on the way to something inside grants nothing to ask from.
		expect(requestableRoles(null)).toEqual([]);
	});
});
