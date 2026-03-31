import { openUrl } from "@tauri-apps/plugin-opener";
import type { AnchorHTMLAttributes } from "react";
import { useDeferredValue } from "react";
import Markdown from "react-markdown";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import type { MessageRole } from "@/types/session";

interface StreamMessageProps {
	content: string;
	role: MessageRole;
}

function ExternalLink(props: AnchorHTMLAttributes<HTMLAnchorElement>) {
	const { href, children, ...rest } = props;
	const handleClick = (e: React.MouseEvent<HTMLAnchorElement>) => {
		e.preventDefault();
		if (href) {
			openUrl(href).catch(() => {});
		}
	};
	return (
		<a {...rest} href={href} onClick={handleClick}>
			{children}
		</a>
	);
}

export function StreamMessage({ content, role }: StreamMessageProps) {
	const isHuman = role === "human";
	const deferredContent = useDeferredValue(content);

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
					<Markdown
						remarkPlugins={remarkPluginList}
						rehypePlugins={rehypePluginList}
						components={{ a: ExternalLink }}
					>
						{deferredContent}
					</Markdown>
				</div>
			)}
		</div>
	);
}
