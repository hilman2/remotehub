import { describe, expect, it } from 'vitest';
import {
	dockerCommands,
	dockerUpdate,
	windowsCommands,
	windowsDownloads,
	windowsUpdate
} from './setup';

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

	it('replaces a running container with the release and keeps token and data', () => {
		const lines = dockerUpdate(origin, '0.4.0').split('\n');
		expect(lines[0]).toBe('sudo docker pull ghcr.io/hilman2/remotehub-connector:0.4.0');
		expect(lines[1]).toBe('sudo docker rm -f remotehub-connector');
		const update = lines.join('\n');
		// The same container as a new one, without asking for the token again.
		expect(update).toContain(dockerCommands(origin, '0.4.0').split('\n').slice(3).join('\n'));
		expect(update).not.toContain('cat >');
		expect(update).toContain('remotehub-connector-data:/var/lib/remotehub-connector');
	});

	it('updates the Windows service from the downloaded file', () => {
		const lines = windowsUpdate().split('\n');
		expect(lines.at(-1)).toBe('.\\remotehub-connector.exe update');
		expect(lines[0]).toContain('Get-FileHash');
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
