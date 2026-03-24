type ToolCategory = "read" | "write" | "command" | "other";

const READ_TOOLS = new Set([
	"Read",
	"Glob",
	"Grep",
	"WebFetch",
	"WebSearch",
	"ListMcpResourcesTool",
	"ReadMcpResourceTool",
	"ToolSearch",
]);

const COMMAND_TOOLS = new Set(["Bash"]);

const WRITE_TOOLS = new Set(["Write", "Edit", "NotebookEdit"]);

export function classifyTool(toolName: string): ToolCategory {
	if (READ_TOOLS.has(toolName)) return "read";
	if (COMMAND_TOOLS.has(toolName)) return "command";
	if (WRITE_TOOLS.has(toolName)) return "write";

	// MCP tools: infer from name patterns
	if (toolName.startsWith("mcp__")) {
		const lower = toolName.toLowerCase();
		if (
			lower.includes("read") ||
			lower.includes("get") ||
			lower.includes("list") ||
			lower.includes("search") ||
			lower.includes("fetch") ||
			lower.includes("retrieve") ||
			lower.includes("query")
		)
			return "read";
		if (
			lower.includes("write") ||
			lower.includes("create") ||
			lower.includes("update") ||
			lower.includes("delete") ||
			lower.includes("edit") ||
			lower.includes("post") ||
			lower.includes("patch") ||
			lower.includes("put")
		)
			return "write";
	}

	return "other";
}

export function shortenPath(fullPath: string, basePath?: string): string {
	if (basePath && fullPath.startsWith(basePath)) {
		const rel = fullPath.slice(basePath.length);
		return rel.startsWith("/") ? rel.slice(1) : rel;
	}
	return fullPath;
}

export function getReadToolLabel(
	toolName: string,
	input: Record<string, unknown>,
	basePath?: string,
): string {
	if (input.file_path && typeof input.file_path === "string") {
		return `Explored ${shortenPath(input.file_path, basePath)}`;
	}
	if (input.pattern && typeof input.pattern === "string") {
		return `Explored ${input.pattern}`;
	}
	if (input.path && typeof input.path === "string") {
		return `Explored ${shortenPath(input.path, basePath)}`;
	}
	if (input.query && typeof input.query === "string") {
		const q = input.query as string;
		return `Searched "${q.length > 60 ? `${q.slice(0, 60)}…` : q}"`;
	}
	if (input.url && typeof input.url === "string") {
		return `Fetched ${input.url}`;
	}
	return `Explored (${toolName})`;
}

export function getCommandLabel(input: Record<string, unknown>): string {
	if (input.command && typeof input.command === "string") {
		const cmd = input.command as string;
		return cmd.length > 80 ? `${cmd.slice(0, 80)}…` : cmd;
	}
	return "command";
}
