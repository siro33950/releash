export function quotePathForShell(path: string): string {
	if (/[ \t'"\\!$`(){}[\]<>|;&*?#~]/.test(path)) {
		return `'${path.replace(/'/g, "'\\''")}'`;
	}
	return path;
}

export function quotePathsForShell(paths: string[]): string {
	return paths.map(quotePathForShell).join(" ");
}
