import { MessageSquarePlus } from "lucide-react";
import { useCallback, useDeferredValue, useMemo } from "react";
import Markdown, { type Components, type Options } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { Button } from "@/components/ui/button";
import { rehypeLineAnnotation } from "@/lib/rehypeLineAnnotation";
import { cn } from "@/lib/utils";

const sanitizeSchema = {
	...defaultSchema,
	attributes: {
		...defaultSchema.attributes,
		code: [...(defaultSchema.attributes?.code ?? []), "className"],
		"*": [...(defaultSchema.attributes?.["*"] ?? []), "dataSourceLine"],
	},
};

interface WorkflowDocumentViewerProps {
	content: string;
	className?: string;
	onCreateThread?: (line: number) => void;
}

function BlockWrapper({
	line,
	onCreateThread,
	children,
}: {
	line: number;
	onCreateThread?: (line: number) => void;
	children: React.ReactNode;
}) {
	const handleClick = useCallback(() => {
		onCreateThread?.(line);
	}, [onCreateThread, line]);

	return (
		<div className="relative group" data-source-line={line}>
			{children}
			{onCreateThread && (
				<Button
					variant="ghost"
					size="icon-xs"
					className="absolute -left-6 top-0.5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity text-muted-foreground hover:text-foreground"
					onClick={handleClick}
					aria-label={`Comment on line ${line}`}
				>
					<MessageSquarePlus className="h-4 w-4" />
				</Button>
			)}
		</div>
	);
}

interface BlockComponentProps {
	node?: { position?: { start: { line: number } } };
	children?: React.ReactNode;
	[key: string]: unknown;
}

function makeBlockComponent(
	Tag: string,
	onCreateThread?: (line: number) => void,
) {
	return function BlockComponent({
		node,
		children,
		...props
	}: BlockComponentProps) {
		const line = node?.position?.start?.line;
		const El = Tag as keyof React.JSX.IntrinsicElements;
		if (line != null) {
			return (
				<BlockWrapper line={line} onCreateThread={onCreateThread}>
					<El {...props}>{children}</El>
				</BlockWrapper>
			);
		}
		return <El {...props}>{children}</El>;
	};
}

function createComponents(
	onCreateThread?: (line: number) => void,
): Partial<Components> {
	return {
		p: makeBlockComponent("p", onCreateThread),
		h1: makeBlockComponent("h1", onCreateThread),
		h2: makeBlockComponent("h2", onCreateThread),
		h3: makeBlockComponent("h3", onCreateThread),
		h4: makeBlockComponent("h4", onCreateThread),
		h5: makeBlockComponent("h5", onCreateThread),
		h6: makeBlockComponent("h6", onCreateThread),
		pre: makeBlockComponent("pre", onCreateThread),
		table: makeBlockComponent("table", onCreateThread),
		blockquote: makeBlockComponent("blockquote", onCreateThread),
	} as Partial<Components>;
}

export function WorkflowDocumentViewer({
	content,
	className,
	onCreateThread,
}: WorkflowDocumentViewerProps) {
	const deferredContent = useDeferredValue(content);
	const remarkPlugins = useMemo(() => [remarkGfm], []);
	const rehypePlugins = useMemo(
		() =>
			[
				rehypeLineAnnotation,
				rehypeRaw,
				[rehypeSanitize, sanitizeSchema],
				rehypeHighlight,
			] as Options["rehypePlugins"],
		[],
	);

	const components = useMemo(
		() => createComponents(onCreateThread),
		[onCreateThread],
	);

	return (
		<div
			data-testid="workflow-document-viewer"
			className={cn(
				"markdown-preview h-full overflow-auto p-6 pl-10 select-text",
				className,
			)}
		>
			<Markdown
				remarkPlugins={remarkPlugins}
				rehypePlugins={rehypePlugins}
				components={components}
			>
				{deferredContent}
			</Markdown>
		</div>
	);
}
