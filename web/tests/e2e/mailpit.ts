import { expect } from '@playwright/test';

/** The lab's Mailpit (#145): it keeps every mail remotehub sends. */
const mailpit = process.env.E2E_MAILPIT_URL ?? 'http://mail:8025';

export interface Mail {
	subject: string;
	text: string;
}

/** The mails to `to`, once at least one has arrived. */
export async function inbox(to: string): Promise<Mail[]> {
	let mails: Mail[] = [];
	await expect(async () => {
		const found = (await (
			await fetch(`${mailpit}/api/v1/search?query=${encodeURIComponent(`to:${to}`)}`)
		).json()) as { messages: { ID: string }[] };
		mails = await Promise.all(
			found.messages.map(async ({ ID }) => {
				const full = (await (await fetch(`${mailpit}/api/v1/message/${ID}`)).json()) as {
					Subject: string;
					Text: string;
				};
				return { subject: full.Subject, text: full.Text };
			})
		);
		expect(mails.length, `a mail to ${to}`).toBeGreaterThan(0);
	}).toPass({ timeout: 20_000 });
	return mails;
}
