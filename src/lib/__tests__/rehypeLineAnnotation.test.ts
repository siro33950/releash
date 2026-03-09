import { describe, expect, it } from "vitest";
import { rehypeLineAnnotation } from "../rehypeLineAnnotation";

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

describe("rehypeLineAnnotation", () => {
	it("adds data-source-line to block elements", () => {
		const tree = makeRoot([
			makeElement("p", 1, 2),
			makeElement("h1", 3, 3),
			makeElement("pre", 5, 8),
		]);

		const plugin = rehypeLineAnnotation();
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin(tree as any);

		const p = tree.children[0] as ReturnType<typeof makeElement>;
		const h1 = tree.children[1] as ReturnType<typeof makeElement>;
		const pre = tree.children[2] as ReturnType<typeof makeElement>;

		expect(p.properties.dataSourceLine).toBe(1);
		expect(h1.properties.dataSourceLine).toBe(3);
		expect(pre.properties.dataSourceLine).toBe(5);
	});

	it("skips inline elements", () => {
		const tree = makeRoot([makeElement("span", 1, 1)]);

		const plugin = rehypeLineAnnotation();
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin(tree as any);

		const span = tree.children[0] as ReturnType<typeof makeElement>;
		expect(span.properties.dataSourceLine).toBeUndefined();
	});

	it("skips elements without position", () => {
		const tree = makeRoot([
			{ type: "element", tagName: "p", properties: {}, children: [] },
		]);

		const plugin = rehypeLineAnnotation();
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin(tree as any);

		const el = tree.children[0] as { properties: { dataSourceLine?: number } };
		expect(el.properties.dataSourceLine).toBeUndefined();
	});

	it("processes nested block elements", () => {
		const tree = makeRoot([makeElement("ul", 1, 5, [makeElement("li", 2, 3)])]);

		const plugin = rehypeLineAnnotation();
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin(tree as any);

		const ul = tree.children[0] as ReturnType<typeof makeElement>;
		const li = ul.children[0] as ReturnType<typeof makeElement>;

		expect(ul.properties.dataSourceLine).toBe(1);
		expect(li.properties.dataSourceLine).toBe(2);
	});

	it("initializes properties if missing", () => {
		const tree = makeRoot([
			{
				type: "element",
				tagName: "p",
				children: [],
				position: { start: { line: 1 }, end: { line: 1 } },
			},
		]);

		const plugin = rehypeLineAnnotation();
		// biome-ignore lint/suspicious/noExplicitAny: test helper
		plugin(tree as any);

		const el = tree.children[0] as { properties: { dataSourceLine?: number } };
		expect(el.properties.dataSourceLine).toBe(1);
	});
});
