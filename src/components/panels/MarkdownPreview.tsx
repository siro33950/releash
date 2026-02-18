import { useDeferredValue, useMemo } from "react";
import Markdown, { type Options } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

const sanitizeSchema = {
	...defaultSchema,
	attributes: {
		...defaultSchema.attributes,
		code: [...(defaultSchema.attributes?.code ?? []), "className"],
	},
};

const rehypePluginList = [
	rehypeRaw,
	[rehypeSanitize, sanitizeSchema],
	rehypeHighlight,
] as Options["rehypePlugins"];

export interface MarkdownPreviewProps {
	content: string;
	className?: string;
}

export function MarkdownPreview({ content, className }: MarkdownPreviewProps) {
	const deferredContent = useDeferredValue(content);
	const plugins = useMemo(() => [remarkGfm], []);

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
