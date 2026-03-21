import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import type { MessageRole } from "@/types/session";

interface StreamMessageProps {
	content: string;
	role: MessageRole;
	isStreaming: boolean;
}

export function StreamMessage({
	content,
	role,
	isStreaming,
}: StreamMessageProps) {
	const isHuman = role === "human";
	const deferredContent = useDeferredValue(content);
	const plugins = useMemo(() => remarkPluginList, []);

	if (role === "system") {
		return (
			<div data-testid="stream-message-system" className="px-4 py-2">
				<div className="bg-muted/60 border border-border/50 text-muted-foreground rounded-md px-3 py-2 text-sm">
					{content}
				</div>
			</div>
		);
	}

	return (
		<div
			data-testid={`stream-message-${role}`}
			className={`${isHuman ? "px-2" : "pt-1 pb-2 px-5"}`}
		>
			{isHuman ? (
				<div className="bg-muted rounded-lg px-3 py-2">
					<p className="text-sm whitespace-pre-wrap break-words">{content}</p>
				</div>
			) : (
				<div className="markdown-preview prose prose-sm dark:prose-invert max-w-none break-words">
					<Markdown remarkPlugins={plugins} rehypePlugins={rehypePluginList}>
						{deferredContent}
					</Markdown>
					{isStreaming && (
						<span className="inline-block w-2 h-4 bg-foreground/60 animate-pulse ml-0.5" />
					)}
				</div>
			)}
		</div>
	);
}
