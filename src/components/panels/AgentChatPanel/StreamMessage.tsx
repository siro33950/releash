import { useDeferredValue, useMemo } from "react";
import Markdown from "react-markdown";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import type { ChatMessage } from "@/types/session";

interface StreamMessageProps {
	message: ChatMessage;
	isStreaming: boolean;
}

export function StreamMessage({ message, isStreaming }: StreamMessageProps) {
	const isHuman = message.role === "human";
	const deferredContent = useDeferredValue(message.content);
	const plugins = useMemo(() => remarkPluginList, []);

	return (
		<div data-testid={`stream-message-${message.role}`} className="px-4 py-3">
			{isHuman && <div className="border-t border-border/50 -mx-4 mb-3" />}
			<div className="text-xs text-muted-foreground mb-1">
				{isHuman ? "User" : "Agent"}
			</div>
			{isHuman ? (
				<p className="text-sm whitespace-pre-wrap break-words">
					{message.content}
				</p>
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
