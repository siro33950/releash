import { describe, expect, it } from "vitest";
import {
	classifyTool,
	getCommandLabel,
	getReadToolLabel,
} from "./toolClassification";

describe("classifyTool", () => {
	it("classifies read tools", () => {
		expect(classifyTool("Read")).toBe("read");
		expect(classifyTool("Glob")).toBe("read");
		expect(classifyTool("Grep")).toBe("read");
		expect(classifyTool("WebFetch")).toBe("read");
		expect(classifyTool("WebSearch")).toBe("read");
	});

	it("classifies command tools", () => {
		expect(classifyTool("Bash")).toBe("command");
	});

	it("classifies write tools", () => {
		expect(classifyTool("Write")).toBe("write");
		expect(classifyTool("Edit")).toBe("write");
		expect(classifyTool("NotebookEdit")).toBe("write");
	});

	it("classifies unknown tools as other", () => {
		expect(classifyTool("Unknown")).toBe("other");
	});

	it("classifies MCP read tools by name pattern", () => {
		expect(classifyTool("mcp__notion__get_page")).toBe("read");
		expect(classifyTool("mcp__server__list_items")).toBe("read");
		expect(classifyTool("mcp__server__search_docs")).toBe("read");
		expect(classifyTool("mcp__server__fetch_data")).toBe("read");
		expect(classifyTool("mcp__server__retrieve_info")).toBe("read");
		expect(classifyTool("mcp__server__read_file")).toBe("read");
		expect(classifyTool("mcp__server__query_data")).toBe("read");
	});

	it("classifies MCP write tools by name pattern", () => {
		expect(classifyTool("mcp__notion__create_page")).toBe("write");
		expect(classifyTool("mcp__server__update_item")).toBe("write");
		expect(classifyTool("mcp__server__delete_record")).toBe("write");
		expect(classifyTool("mcp__server__post_data")).toBe("write");
		expect(classifyTool("mcp__server__patch_item")).toBe("write");
	});

	it("classifies MCP tools without matching pattern as other", () => {
		expect(classifyTool("mcp__server__run_something")).toBe("other");
	});
});

describe("getReadToolLabel", () => {
	it("returns file_path label", () => {
		expect(getReadToolLabel("Read", { file_path: "/src/main.ts" })).toBe(
			"Explored /src/main.ts",
		);
	});

	it("returns pattern label", () => {
		expect(getReadToolLabel("Glob", { pattern: "**/*.ts" })).toBe(
			"Explored **/*.ts",
		);
	});

	it("returns path label", () => {
		expect(getReadToolLabel("Grep", { path: "/src", pattern: "foo" })).toBe(
			"Explored foo",
		);
	});

	it("returns query label for search", () => {
		expect(getReadToolLabel("WebSearch", { query: "react hooks" })).toBe(
			'Searched "react hooks"',
		);
	});

	it("truncates long query", () => {
		const longQuery = "a".repeat(100);
		const label = getReadToolLabel("WebSearch", { query: longQuery });
		expect(label).toContain("…");
		expect(label.length).toBeLessThan(100);
	});

	it("returns url label for fetch", () => {
		expect(getReadToolLabel("WebFetch", { url: "https://example.com" })).toBe(
			"Fetched https://example.com",
		);
	});

	it("returns fallback label", () => {
		expect(getReadToolLabel("Read", {})).toBe("Explored (Read)");
	});
});

describe("getCommandLabel", () => {
	it("returns command string", () => {
		expect(getCommandLabel({ command: "git status" })).toBe("git status");
	});

	it("truncates long command", () => {
		const longCmd = "a".repeat(100);
		const label = getCommandLabel({ command: longCmd });
		expect(label.endsWith("…")).toBe(true);
		expect(label.length).toBe(81);
	});

	it("returns fallback for no command", () => {
		expect(getCommandLabel({})).toBe("command");
	});
});
