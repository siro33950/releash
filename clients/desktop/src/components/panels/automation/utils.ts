export function extractTemplateVariables(content: string): string[] {
	const matches = content.match(/\{\{([^}]+)\}\}/g);
	if (!matches) return [];
	return [
		...new Set(matches.map((m) => m.slice(2, -2).trim()).filter((v) => v)),
	];
}
