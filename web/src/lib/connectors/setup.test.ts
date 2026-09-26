import { describe, expect, it } from 'vitest';
import { dockerCommands, windowsCommands, windowsDownloads } from './setup';

describe('starting a connector', () => {
	const origin = 'https://remotehub.example.com';

	it('runs the image of the running release against this remotehub', () => {
		const commands = dockerCommands(origin, '0.3.0');
		expect(commands).toContain('ghcr.io/hilman2/remotehub-connector:0.3.0');
		expect(commands).toContain(`REMOTEHUB_URL=${origin} `);
		// Every line but the last continues the docker run command or stands alone.
		const run = commands.split('\n').slice(3);
		expect(run.slice(0, -1).every((line) => line.endsWith(' \\'))).toBe(true);
	});

	it('installs the Windows service for this remotehub', () => {
		expect(windowsCommands(origin)).toContain(`install --url ${origin}`);
		expect(windowsDownloads('0.3.0')).toEqual({
			program:
				'https://github.com/hilman2/remotehub/releases/download/v0.3.0/remotehub-connector.exe',
			sums: 'https://github.com/hilman2/remotehub/releases/download/v0.3.0/SHA256SUMS'
		});
	});
});
