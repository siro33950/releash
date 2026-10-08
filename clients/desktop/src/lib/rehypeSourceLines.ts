import type { DiffRange } from "@/types/markdown-diff";

interface HastPosition {
	start: { line: number };
	end: { line: number };
}

interface HastProperties {
	className?: string[];
	[key: string]: unknown;
}

interface HastNode {
	type: string;
	tagName?: string;
	properties?: HastProperties;
	children?: HastNode[];
	position?: HastPosition;
}

const BLOCK_TAGS = new Set([
	"p",
	"h1",
	"h2",
	"h3",
	"h4",
	"h5",
	"h6",
	"pre",
	"table",
	"ul",
	"ol",
	"blockquote",
	"hr",
	"li",
	"tr",
]);

function rangesOverlap(
	aStart: number,
	aEnd: number,
	bStart: number,
	bEnd: number,
): boolean {
	return aStart <= bEnd && bStart <= aEnd;
}

function findMatchingRange(
	nodeStart: number,
	nodeEnd: number,
	diffRanges: DiffRange[],
): DiffRange | undefined {
	return diffRanges.find((r) =>
		rangesOverlap(nodeStart, nodeEnd, r.startLine, r.endLine),
	);
}

function visitBlock(
	node: HastNode,
	diffRanges: DiffRange[],
	classPrefix: string,
): void {
	if (
		node.type === "element" &&
		node.tagName &&
		BLOCK_TAGS.has(node.tagName) &&
		node.position
	) {
		const match = findMatchingRange(
			node.position.start.line,
			node.position.end.line,
			diffRanges,
		);
		if (match) {
			if (!node.properties) {
				node.properties = {};
			}
			const className = node.properties.className ?? [];
			className.push(`${classPrefix}-${match.type}`);
			node.properties.className = className;
		}
	}

	if (node.children) {
		for (const child of node.children) {
			visitBlock(child, diffRanges, classPrefix);
		}
	}
}

export function rehypeSourceLines(
	diffRanges: DiffRange[],
	classPrefix = "md-diff-gutter",
) {
	return () => (tree: HastNode) => {
		visitBlock(tree, diffRanges, classPrefix);
	};
}
