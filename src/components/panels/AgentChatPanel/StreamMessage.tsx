import { openUrl } from "@tauri-apps/plugin-opener";
import type { AnchorHTMLAttributes } from "react";
import { useDeferredValue } from "react";
import Markdown from "react-markdown";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import type { ImagePart, MentionReference, MessageRole } from "@/types/session";

interface DisplayPart {
	type: "text" | "mention";
	value: string;
}

interface StreamMessageProps {
	content: string;
	role: MessageRole;
	images?: ImagePart[];
	mentions?: MentionReference[];
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

function buildDisplayParts(
	content: string,
	mentions?: MentionReference[],
): DisplayPart[] {
	if (!mentions || mentions.length === 0) {
		return content ? [{ type: "text", value: content }] : [];
	}
	const parts: DisplayPart[] = [];
	let remaining = content;
	for (const mention of mentions) {
		const mentionText = `@${mention.filePath}`;
		const idx = remaining.indexOf(mentionText);
		if (idx === -1) continue;
		let matchEnd = idx + mentionText.length;
		const after = remaining.slice(matchEnd);
		const lineMatch = after.match(/^:L(\d+)(?:-L(\d+))?/);
		if (lineMatch) matchEnd += lineMatch[0].length;
		if (idx > 0) parts.push({ type: "text", value: remaining.slice(0, idx) });
		parts.push({
			type: "mention",
			value: remaining.slice(idx, matchEnd),
		});
		remaining = remaining.slice(matchEnd);
	}
	if (remaining) parts.push({ type: "text", value: remaining });
	return parts;
}

function HumanMessageContent({
	content,
	images,
	mentions,
}: {
	content: string;
	images?: ImagePart[];
	mentions?: MentionReference[];
}) {
	const displayParts = buildDisplayParts(content, mentions);

	const imageElements =
		images && images.length > 0
			? images.map((img, index) => (
					<img
						// biome-ignore lint/suspicious/noArrayIndexKey: images are positional data, order is fixed
						key={`${index}-${img.mediaType}-${img.data.slice(0, 20)}`}
						src={`data:${img.mediaType};base64,${img.data}`}
						alt="Attached"
						className="max-h-48 max-w-full rounded-md"
					/>
				))
			: null;

	return (
		<div className="bg-muted rounded-lg px-3 py-2">
			{imageElements && imageElements.length > 0 && (
				<div className="flex flex-wrap gap-2 mb-2">{imageElements}</div>
			)}
			<p className="text-sm whitespace-pre-wrap break-words">
				{displayParts.map((part, i) => {
					const key = `${part.type}:${i}`;
					return part.type === "mention" ? (
						<span
							key={key}
							className="inline-flex items-center bg-primary/10 text-primary rounded px-1.5 py-0.5 text-xs font-mono"
						>
							{part.value}
						</span>
					) : (
						<span key={key}>{part.value}</span>
					);
				})}
			</p>
		</div>
	);
}

export function StreamMessage({
	content,
	role,
	images,
	mentions,
}: StreamMessageProps) {
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
				<HumanMessageContent
					content={content}
					images={images}
					mentions={mentions}
				/>
			) : (
				<div className="markdown-preview prose prose-sm dark:prose-invert max-w-none break-words">
					<Markdown
						remarkPlugins={remarkPluginList}
						rehypePlugins={rehypePluginList}
						components={{
							a: ExternalLink,
							table: ({ children: c, ...props }) => (
								<ScrollArea className="max-w-full">
									<table {...props}>{c}</table>
									<ScrollBar orientation="horizontal" />
								</ScrollArea>
							),
							pre: ({ children: c, ...props }) => (
								<ScrollArea className="max-w-full">
									<pre {...props}>{c}</pre>
									<ScrollBar orientation="horizontal" />
								</ScrollArea>
							),
						}}
					>
						{deferredContent}
					</Markdown>
				</div>
			)}
		</div>
	);
}
