import { describe, expect, it } from "vitest";
import { isMarkdownFile } from "../markdownUtils";

describe("isMarkdownFile", () => {
	it.each(["readme.md", "doc.mdx"])("returns true for %s", (path) => {
		expect(isMarkdownFile(path)).toBe(true);
	});

	it.each([
		"file.ts",
		"data.json",
		"style.css",
		"noext",
		"image.png",
	])("returns false for %s", (path) => {
		expect(isMarkdownFile(path)).toBe(false);
	});

	it("is case-insensitive", () => {
		expect(isMarkdownFile("README.MD")).toBe(true);
		expect(isMarkdownFile("doc.MDX")).toBe(true);
	});

	it("handles paths with directories", () => {
		expect(isMarkdownFile("/Users/foo/bar/README.md")).toBe(true);
		expect(isMarkdownFile("docs/guide.mdx")).toBe(true);
	});
});
