interface HastPosition {
	start: { line: number };
	end: { line: number };
}

interface HastProperties {
	className?: string[];
	dataSourceLine?: number;
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

function visitBlock(node: HastNode): void {
	if (
		node.type === "element" &&
		node.tagName &&
		BLOCK_TAGS.has(node.tagName) &&
		node.position
	) {
		if (!node.properties) {
			node.properties = {};
		}
		node.properties.dataSourceLine = node.position.start.line;
	}

	if (node.children) {
		for (const child of node.children) {
			visitBlock(child);
		}
	}
}

export function rehypeLineAnnotation() {
	return (tree: HastNode) => {
		visitBlock(tree);
	};
}
