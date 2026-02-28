const ANSI_ESCAPE_RE =
	// biome-ignore lint/suspicious/noControlCharactersInRegex: intentional ANSI escape removal
	/\u001B\][^\u0007\u001B]*(?:\u0007|\u001B\\)|\u001B\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]|\u009B[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]|\u001B[\x20-\x2f]+[\x30-\x7e]|\u001B[\x30-\x7e]/g;

const CONTROL_CHAR_RE =
	// biome-ignore lint/suspicious/noControlCharactersInRegex: intentional control char removal
	/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;

export function stripAnsi(text: string): string {
	let result = text.replace(ANSI_ESCAPE_RE, "");
	result = processCarriageReturns(result);
	result = result.replace(CONTROL_CHAR_RE, "");
	return result;
}

function processCarriageReturns(text: string): string {
	return text
		.split("\n")
		.map((line) => {
			if (!line.includes("\r")) return line;
			const parts = line.split("\r");
			let current = "";
			for (const part of parts) {
				if (part.length === 0) continue;
				current = part + current.slice(part.length);
			}
			return current;
		})
		.join("\n");
}
