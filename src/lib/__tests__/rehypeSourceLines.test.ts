import { describe, expect, it } from "vitest";
import type { DiffRange } from "@/types/markdown-diff";
import { rehypeSourceLines } from "../rehypeSourceLines";

function makeElement(
	tagName: string,
	startLine: number,
	endLine: number,
	children: unknown[] = [],
	properties: Record<string, unknown> = {},
) {
	return {
		type: "element" as const,
		tagName,
		properties: { ...properties },
		children,
		position: {
			start: { line: startLine, column: 1, offset: 0 },
			end: { line: endLine, column: 1, offset: 0 },
		},
	};
}

function makeRoot(children: unknown[]) {
	return {
		type: "root" as const,
		children,
	};
}

describe("rehypeSourceLines", () => {
	it("adds md-diff-gutter-added class to matching block elements", () => {
		const ranges: DiffRange[] = [{ startLine: 3, endLine: 5, type: "added" }];
		const tree = makeRoot([
			makeElement("p", 1, 2),
			makeElement("p", 3, 5),
			makeElement("p", 6, 8),
		]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		expect(tree.children[0]).not.toHaveProperty(
			"properties.className",
			expect.arrayContaining([expect.stringMatching(/^md-diff-gutter-/)]),
		);
		expect(tree.children[1]).toHaveProperty("properties.className", [
			"md-diff-gutter-added",
		]);
		expect(tree.children[2]).not.toHaveProperty(
			"properties.className",
			expect.arrayContaining([expect.stringMatching(/^md-diff-gutter-/)]),
		);
	});

	it("adds md-diff-gutter-modified class for modified ranges", () => {
		const ranges: DiffRange[] = [
			{ startLine: 1, endLine: 2, type: "modified" },
		];
		const tree = makeRoot([makeElement("h1", 1, 1)]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		expect(tree.children[0]).toHaveProperty("properties.className", [
			"md-diff-gutter-modified",
		]);
	});

	it("handles partial overlap", () => {
		const ranges: DiffRange[] = [{ startLine: 2, endLine: 3, type: "added" }];
		const tree = makeRoot([makeElement("p", 1, 3)]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		expect(tree.children[0]).toHaveProperty("properties.className", [
			"md-diff-gutter-added",
		]);
	});

	it("skips non-block elements", () => {
		const ranges: DiffRange[] = [{ startLine: 1, endLine: 1, type: "added" }];
		const tree = makeRoot([makeElement("span", 1, 1)]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		const el = tree.children[0] as ReturnType<typeof makeElement>;
		const classes = el.properties.className;
		expect(classes).toBeUndefined();
	});

	it("handles elements without position gracefully", () => {
		const ranges: DiffRange[] = [{ startLine: 1, endLine: 1, type: "added" }];
		const tree = makeRoot([
			{ type: "element", tagName: "p", properties: {}, children: [] },
		]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		const el = tree.children[0] as { properties: { className?: string[] } };
		expect(el.properties.className).toBeUndefined();
	});

	it("processes nested block elements", () => {
		const ranges: DiffRange[] = [{ startLine: 2, endLine: 2, type: "added" }];
		const tree = makeRoot([makeElement("ul", 1, 3, [makeElement("li", 2, 2)])]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		const ul = tree.children[0] as ReturnType<typeof makeElement>;
		const li = ul.children[0] as ReturnType<typeof makeElement>;
		expect(ul.properties.className).toContain("md-diff-gutter-added");
		expect(li.properties.className).toContain("md-diff-gutter-added");
	});

	it("does nothing with empty diff ranges", () => {
		const tree = makeRoot([makeElement("p", 1, 3)]);

		const plugin = rehypeSourceLines([]);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		const el = tree.children[0] as ReturnType<typeof makeElement>;
		const classes = el.properties.className;
		expect(classes).toBeUndefined();
	});

	it("preserves existing className", () => {
		const ranges: DiffRange[] = [{ startLine: 1, endLine: 1, type: "added" }];
		const tree = makeRoot([
			makeElement("p", 1, 1, [], { className: ["existing"] }),
		]);

		const plugin = rehypeSourceLines(ranges);
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		expect(tree.children[0]).toHaveProperty("properties.className", [
			"existing",
			"md-diff-gutter-added",
		]);
	});

	it("uses custom classPrefix when provided", () => {
		const ranges: DiffRange[] = [{ startLine: 1, endLine: 1, type: "added" }];
		const tree = makeRoot([makeElement("p", 1, 1)]);

		const plugin = rehypeSourceLines(ranges, "md-diff-split");
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		expect(tree.children[0]).toHaveProperty("properties.className", [
			"md-diff-split-added",
		]);
	});

	it("supports deleted type with custom prefix", () => {
		const ranges: DiffRange[] = [{ startLine: 1, endLine: 2, type: "deleted" }];
		const tree = makeRoot([makeElement("p", 1, 2)]);

		const plugin = rehypeSourceLines(ranges, "md-diff-split");
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin()(tree as any);

		expect(tree.children[0]).toHaveProperty("properties.className", [
			"md-diff-split-deleted",
		]);
	});
});
