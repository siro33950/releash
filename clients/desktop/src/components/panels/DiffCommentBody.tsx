import Markdown from "react-markdown";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import { cn } from "@/lib/utils";

export interface DiffCommentBodyProps {
	content: string;
	className?: string;
}

export function DiffCommentBody({ content, className }: DiffCommentBodyProps) {
	return (
		<div
			data-testid="diff-comment-body"
			className={cn(
				"markdown-preview markdown-preview-comment break-words",
				className,
			)}
		>
			<Markdown
				remarkPlugins={remarkPluginList}
				rehypePlugins={rehypePluginList}
			>
				{content}
			</Markdown>
		</div>
	);
}
