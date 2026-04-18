const MARKDOWN_EXTENSIONS = new Set(["md", "mdx", "markdown"]);

export function isMarkdownFile(path: string): boolean {
	const ext = path.split(".").pop()?.toLowerCase() ?? "";
	return MARKDOWN_EXTENSIONS.has(ext);
}
