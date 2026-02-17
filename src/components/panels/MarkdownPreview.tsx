import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

export interface MarkdownPreviewProps {
	content: string;
	className?: string;
}

export function MarkdownPreview({ content, className }: MarkdownPreviewProps) {
	const deferredContent = useDeferredValue(content);
	const plugins = useMemo(() => [remarkGfm], []);
	const rehypePlugins = useMemo(() => [rehypeHighlight], []);

	return (
		<div
			data-testid="markdown-preview"
			className={cn(
				"markdown-preview h-full overflow-auto p-6 scrollbar-thin",
				className,
			)}
		>
			<Markdown remarkPlugins={plugins} rehypePlugins={rehypePlugins}>
				{deferredContent}
			</Markdown>
		</div>
	);
}
