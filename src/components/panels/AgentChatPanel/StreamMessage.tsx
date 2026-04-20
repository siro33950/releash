import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AnchorHTMLAttributes } from "react";
import { useDeferredValue, useEffect, useState } from "react";
import Markdown from "react-markdown";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import type { ImagePart, MessageRole } from "@/types/session";

interface DisplayPart {
	type: "text" | "mention";
	value: string;
}

interface StreamMessageProps {
	content: string;
	role: MessageRole;
	images?: ImagePart[];
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

const displayPartsCache = new Map<string, DisplayPart[]>();

function HumanMessageContent({
	content,
	images,
}: {
	content: string;
	images?: ImagePart[];
}) {
	const [parts, setParts] = useState<DisplayPart[] | null>(
		() => displayPartsCache.get(content) ?? null,
	);

	useEffect(() => {
		const cached = displayPartsCache.get(content);
		if (cached) {
			setParts(cached);
			return;
		}

		let cancelled = false;
		invoke<DisplayPart[]>("parse_display_mentions", { content })
			.then((result) => {
				if (!cancelled) {
					displayPartsCache.set(content, result);
					setParts(result);
				}
			})
			.catch(() => {
				if (!cancelled) {
					const fallback: DisplayPart[] = [{ type: "text", value: content }];
					displayPartsCache.set(content, fallback);
					setParts(fallback);
				}
			});
		return () => {
			cancelled = true;
		};
	}, [content]);

	const displayParts = parts ?? [{ type: "text" as const, value: content }];

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

export function StreamMessage({ content, role, images }: StreamMessageProps) {
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
				<HumanMessageContent content={content} images={images} />
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
						}}
					>
						{deferredContent}
					</Markdown>
				</div>
			)}
		</div>
	);
}
