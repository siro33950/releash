export function generateNotionBranchName(
	branchNameProperty: string,
	pageId?: string,
): string {
	const sanitized = branchNameProperty
		.trim()
		.replace(/\s+/g, "-")
		.replace(/[^a-zA-Z0-9/_-]/g, "")
		.replace(/-{2,}/g, "-")
		.replace(/^[-/]+|[-/]+$/g, "");

	if (sanitized) return sanitized;

	if (pageId) {
		const shortId = pageId.replace(/-/g, "").slice(0, 8);
		return `notion/${shortId}`;
	}

	return "notion-task";
}
