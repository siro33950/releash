import { useVirtualizer } from "@tanstack/react-virtual";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy } from "lucide-react";
import type { AnchorHTMLAttributes } from "react";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import type { ImagePart, MentionReference, MessageRole } from "@/types/session";

const LARGE_AGENT_MESSAGE_CHARS = 12_000;
const LARGE_AGENT_MESSAGE_LINES = 240;
const LARGE_AGENT_MESSAGE_PREVIEW_CHARS = 4_000;
const LARGE_HUMAN_MESSAGE_CHARS = 3_000;
const LARGE_HUMAN_MESSAGE_LINES = 50;
const LARGE_HUMAN_MESSAGE_PREVIEW_CHARS = 1_200;
const VIRTUALIZED_AGENT_MESSAGE_LINES = 600;
const VIRTUALIZED_LINE_HEIGHT = 20;

interface DisplayPart {
	type: "text" | "mention";
	value: string;
}

interface StreamMessageProps {
	content: string;
	role: MessageRole;
	images?: ImagePart[];
	mentions?: MentionReference[];
	rawMode?: boolean;
	timestamp?: number;
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

function formatMentionToken(mention: MentionReference): string {
	const fileToken = /[\s"]/.test(mention.filePath)
		? `@"${mention.filePath.replace(/["\\]/g, "\\$&")}"`
		: `@${mention.filePath}`;
	if (mention.startLine && mention.endLine) {
		return `${fileToken}:L${mention.startLine}-L${mention.endLine}`;
	}
	if (mention.startLine) {
		return `${fileToken}:L${mention.startLine}`;
	}
	return fileToken;
}

function buildDisplayParts(
	content: string,
	mentions?: MentionReference[],
): DisplayPart[] {
	if (!mentions || mentions.length === 0) {
		return content ? [{ type: "text", value: content }] : [];
	}
	const tokens = mentions.map(formatMentionToken);
	const parts: DisplayPart[] = [];
	let cursor = 0;
	while (cursor < content.length) {
		let best: { idx: number; token: string } | null = null;
		for (const token of tokens) {
			const idx = content.indexOf(token, cursor);
			if (idx === -1) continue;
			if (
				!best ||
				idx < best.idx ||
				(idx === best.idx && token.length > best.token.length)
			) {
				best = { idx, token };
			}
		}
		if (!best) {
			parts.push({ type: "text", value: content.slice(cursor) });
			break;
		}
		if (best.idx > cursor) {
			parts.push({ type: "text", value: content.slice(cursor, best.idx) });
		}
		parts.push({ type: "mention", value: best.token });
		cursor = best.idx + best.token.length;
	}
	return parts;
}

export function formatMessageTime(timestamp?: number): string | null {
	if (typeof timestamp !== "number" || !Number.isFinite(timestamp)) return null;
	const ms = timestamp < 1_000_000_000_000 ? timestamp * 1000 : timestamp;
	const date = new Date(ms);
	if (Number.isNaN(date.getTime())) return null;
	return date.toLocaleTimeString([], {
		hour: "2-digit",
		minute: "2-digit",
	});
}

export function MessageCopyButton({
	content,
	ariaLabel = "Copy message",
}: {
	content: string;
	ariaLabel?: string;
}) {
	const [copyState, setCopyState] = useState<"idle" | "copied" | "error">(
		"idle",
	);
	useEffect(() => {
		if (copyState === "idle") return;
		const timeout = window.setTimeout(() => setCopyState("idle"), 1400);
		return () => window.clearTimeout(timeout);
	}, [copyState]);

	const handleCopy = async () => {
		if (!content) return;
		try {
			await navigator.clipboard.writeText(content);
			setCopyState("copied");
		} catch {
			setCopyState("error");
		}
	};

	return (
		<button
			type="button"
			className="inline-flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground/70 hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
			aria-label={ariaLabel}
			title={copyState === "error" ? "Copy failed" : ariaLabel}
			onClick={handleCopy}
			disabled={!content}
		>
			{copyState === "copied" ? (
				<Check className="size-3" />
			) : (
				<Copy className="size-3" />
			)}
		</button>
	);
}

function HumanMessageContent({
	content,
	images,
	mentions,
	timestamp,
}: {
	content: string;
	images?: ImagePart[];
	mentions?: MentionReference[];
	timestamp?: number;
}) {
	const [isExpanded, setIsExpanded] = useState(false);
	const lines = lineCount(content);
	const shouldCollapse =
		content.length > LARGE_HUMAN_MESSAGE_CHARS ||
		lines > LARGE_HUMAN_MESSAGE_LINES;
	const visibleContent =
		shouldCollapse && !isExpanded
			? content.slice(0, LARGE_HUMAN_MESSAGE_PREVIEW_CHARS)
			: content;
	const displayParts = buildDisplayParts(visibleContent, mentions);
	const formattedTime = formatMessageTime(timestamp);

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
		<div className="rounded-lg border border-border bg-background px-3 py-2">
			{imageElements && imageElements.length > 0 && (
				<div className="flex flex-wrap gap-2 mb-2">{imageElements}</div>
			)}
			{shouldCollapse && !isExpanded && (
				<div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
					<span>
						Long message: {content.length.toLocaleString()} chars /{" "}
						{lines.toLocaleString()} lines
					</span>
					<button
						type="button"
						className="inline-flex h-6 items-center rounded border border-border bg-background px-2 text-foreground hover:bg-muted"
						onClick={() => setIsExpanded(true)}
					>
						Show full message
					</button>
				</div>
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
				{shouldCollapse && !isExpanded && content.length > visibleContent.length
					? "\n..."
					: ""}
			</p>
			<div className="mt-1.5 flex items-center justify-end gap-1 text-[11px] text-muted-foreground">
				{formattedTime && <span>{formattedTime}</span>}
				<MessageCopyButton content={content} ariaLabel="Copy human message" />
			</div>
		</div>
	);
}

function lineCount(content: string): number {
	if (!content) return 0;
	return content.split("\n").length;
}

function shouldCollapseAgentMessage(content: string): boolean {
	return (
		content.length > LARGE_AGENT_MESSAGE_CHARS ||
		lineCount(content) > LARGE_AGENT_MESSAGE_LINES
	);
}

export function AgentMarkdown({ content }: { content: string }) {
	return (
		<div className="markdown-preview prose prose-sm dark:prose-invert max-w-none break-words">
			<Markdown
				remarkPlugins={remarkPluginList}
				rehypePlugins={rehypePluginList}
				components={{
					a: ExternalLink,
					table: ({ children: c, ...props }) => (
						<div className="max-w-full overflow-x-auto">
							<table {...props}>{c}</table>
						</div>
					),
					pre: ({ children: c, ...props }) => (
						<div className="max-w-full overflow-x-auto">
							<pre {...props}>{c}</pre>
						</div>
					),
				}}
			>
				{content}
			</Markdown>
		</div>
	);
}

function VirtualizedAgentLines({ content }: { content: string }) {
	const parentRef = useRef<HTMLDivElement | null>(null);
	const lines = useMemo(() => content.split("\n"), [content]);
	const virtualizer = useVirtualizer({
		count: lines.length,
		getScrollElement: () => parentRef.current,
		estimateSize: () => VIRTUALIZED_LINE_HEIGHT,
		overscan: 12,
	});

	return (
		<div
			className="rounded border border-border bg-muted/20 text-sm"
			data-testid="large-agent-message-virtualized"
		>
			<div className="border-b border-border px-3 py-2 text-xs text-muted-foreground">
				Virtualized full message: {lines.length.toLocaleString()} lines
			</div>
			<div
				ref={parentRef}
				className="max-h-[70vh] overflow-auto"
				data-testid="large-agent-message-virtual-scroll"
			>
				<div
					className="relative font-mono text-xs"
					style={{ height: virtualizer.getTotalSize() }}
				>
					{virtualizer.getVirtualItems().map((virtualItem) => (
						<div
							key={virtualItem.key}
							data-index={virtualItem.index}
							className="absolute left-0 top-0 flex h-5 w-full gap-3 whitespace-pre px-3 py-0.5"
							style={{
								transform: `translateY(${virtualItem.start}px)`,
							}}
						>
							<span className="w-12 shrink-0 select-none text-right text-muted-foreground/70">
								{virtualItem.index + 1}
							</span>
							<span className="min-w-0 flex-1 overflow-hidden text-ellipsis">
								{lines[virtualItem.index] || " "}
							</span>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

function RawAgentMessageContent({ content }: { content: string }) {
	const lines = lineCount(content);
	if (lines > VIRTUALIZED_AGENT_MESSAGE_LINES) {
		return <VirtualizedAgentLines content={content} />;
	}
	return (
		<pre
			className="max-h-[70vh] overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-muted/20 px-3 py-2 font-mono text-xs text-foreground"
			data-testid="agent-raw-message"
		>
			{content}
		</pre>
	);
}

function AgentMessageContent({
	content,
	rawMode = false,
}: {
	content: string;
	rawMode?: boolean;
}) {
	const [isExpanded, setIsExpanded] = useState(false);
	const lines = lineCount(content);
	if (rawMode) {
		return <RawAgentMessageContent content={content} />;
	}
	if (!shouldCollapseAgentMessage(content)) {
		return <AgentMarkdown content={content} />;
	}
	if (isExpanded) {
		if (lines > VIRTUALIZED_AGENT_MESSAGE_LINES) {
			return <VirtualizedAgentLines content={content} />;
		}
		return <AgentMarkdown content={content} />;
	}
	const preview = content.slice(0, LARGE_AGENT_MESSAGE_PREVIEW_CHARS);
	return (
		<div
			className="rounded border border-border bg-muted/30 px-3 py-2 text-sm"
			data-testid="large-agent-message-collapsed"
		>
			<div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
				<span>
					Large message collapsed: {content.length.toLocaleString()} chars /{" "}
					{lines.toLocaleString()} lines
				</span>
				<button
					type="button"
					className="inline-flex h-6 items-center rounded border border-border bg-background px-2 text-foreground hover:bg-muted"
					onClick={() => setIsExpanded(true)}
				>
					Show full message
				</button>
			</div>
			<pre className="max-h-64 overflow-y-auto whitespace-pre-wrap break-words text-xs text-muted-foreground">
				{preview}
				{content.length > preview.length ? "\n..." : ""}
			</pre>
		</div>
	);
}

function StreamMessageImpl({
	content,
	role,
	images,
	mentions,
	rawMode = false,
	timestamp,
}: StreamMessageProps) {
	const isHuman = role === "human";

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
			className={`${isHuman ? "flex justify-end px-4 py-1" : "pt-1 pb-2 px-5"}`}
		>
			{isHuman ? (
				<div className="max-w-[min(82%,48rem)]">
					<HumanMessageContent
						content={content}
						images={images}
						mentions={mentions}
						timestamp={timestamp}
					/>
				</div>
			) : (
				<AgentMessageContent content={content} rawMode={rawMode} />
			)}
		</div>
	);
}

function shallowEqualImages(a?: ImagePart[], b?: ImagePart[]): boolean {
	if (a === b) return true;
	if (!a || !b) return false;
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		if (a[i].data !== b[i].data || a[i].mediaType !== b[i].mediaType)
			return false;
	}
	return true;
}

function shallowEqualMentions(
	a?: MentionReference[],
	b?: MentionReference[],
): boolean {
	if (a === b) return true;
	if (!a || !b) return false;
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		const x = a[i];
		const y = b[i];
		if (
			x.filePath !== y.filePath ||
			x.startLine !== y.startLine ||
			x.endLine !== y.endLine
		)
			return false;
	}
	return true;
}

// memoize on (content, role, images, mentions) — the four props that change
// the rendered DOM. Skipping re-render when none of these differ is what
// keeps streaming-heavy turns from re-parsing every sibling message's markdown.
export const StreamMessage = memo(StreamMessageImpl, (prev, next) => {
	return (
		prev.content === next.content &&
		prev.role === next.role &&
		prev.rawMode === next.rawMode &&
		prev.timestamp === next.timestamp &&
		shallowEqualImages(prev.images, next.images) &&
		shallowEqualMentions(prev.mentions, next.mentions)
	);
});
