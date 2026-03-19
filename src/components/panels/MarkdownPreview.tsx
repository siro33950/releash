import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import { cn } from "@/lib/utils";

export interface MarkdownPreviewProps {
	content: string;
	className?: string;
}

export function MarkdownPreview({ content, className }: MarkdownPreviewProps) {
	const deferredContent = useDeferredValue(content);
	const plugins = useMemo(() => remarkPluginList, []);

	return (
		<div
			data-testid="markdown-preview"
			className={cn(
				"markdown-preview h-full overflow-auto p-6 select-text",
				className,
			)}
		>
			<Markdown remarkPlugins={plugins} rehypePlugins={rehypePluginList}>
				{deferredContent}
			</Markdown>
		</div>
	);
}
