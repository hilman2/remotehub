// The connector's web interface (crates/connector/src/ui.rs) works without
// this script. It shows times and sizes in the browser's language and time
// zone, and tells the server the zone of the time in the "open until" form.

const lang = document.documentElement.lang;

const when = new Intl.DateTimeFormat(lang, { dateStyle: 'medium', timeStyle: 'short' });
for (const element of document.querySelectorAll('time[datetime]')) {
	element.textContent = when.format(new Date(element.dateTime));
}

const UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'];
for (const element of document.querySelectorAll('data.bytes')) {
	let value = Number(element.value);
	let unit = 0;
	while (value >= 1000 && unit < UNITS.length - 1) {
		value /= 1000;
		unit += 1;
	}
	element.textContent = new Intl.NumberFormat(lang, {
		style: 'unit',
		unit: UNITS[unit],
		unitDisplay: 'short',
		maximumFractionDigits: unit === 0 ? 0 : 1
	}).format(value);
}

// The offset at the chosen time, not today's: daylight saving time may
// begin or end in between.
for (const form of document.querySelectorAll('form.until')) {
	form.addEventListener('submit', () => {
		const chosen = new Date(form.elements.until.value);
		form.elements.offset.value = String(-chosen.getTimezoneOffset());
	});
}
