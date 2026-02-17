import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { computeModifiedDiffRanges } from "@/lib/markdownDiff";
import { rehypeSourceLines } from "@/lib/rehypeSourceLines";

export interface MarkdownDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
}

export function MarkdownDiffViewer({
	originalContent,
	modifiedContent,
}: MarkdownDiffViewerProps) {
	const deferredOriginal = useDeferredValue(originalContent);
	const deferredModified = useDeferredValue(modifiedContent);

	const diffRanges = useMemo(
		() => computeModifiedDiffRanges(deferredOriginal, deferredModified),
		[deferredOriginal, deferredModified],
	);

	const remarkPlugins = useMemo(() => [remarkGfm], []);
	const rehypePlugins = useMemo(
		() => [rehypeSourceLines(diffRanges), rehypeHighlight],
		[diffRanges],
	);

	return (
		<div
			data-testid="markdown-diff-viewer"
			className="markdown-preview h-full overflow-auto p-6 scrollbar-thin"
		>
			<Markdown remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins}>
				{deferredModified}
			</Markdown>
		</div>
	);
}
