export function generateNotionBranchName(
	branchNameProperty: string,
	pageId?: string,
	prefix?: string,
): string {
	const sanitized = branchNameProperty
		.trim()
		.replace(/\s+/g, "-")
		.replace(/[^a-zA-Z0-9/_-]/g, "")
		.replace(/-{2,}/g, "-")
		.replace(/^[-/]+|[-/]+$/g, "");

	if (sanitized) {
		if (prefix && !sanitized.startsWith(prefix)) {
			return `${prefix}${sanitized}`;
		}
		return sanitized;
	}

	if (pageId) {
		const shortId = pageId.replace(/-/g, "").slice(0, 8);
		const fallback = `notion/${shortId}`;
		if (prefix && !fallback.startsWith(prefix)) {
			return `${prefix}${fallback}`;
		}
		return fallback;
	}

	const fallback = "notion-task";
	if (prefix && !fallback.startsWith(prefix)) {
		return `${prefix}${fallback}`;
	}
	return fallback;
}
