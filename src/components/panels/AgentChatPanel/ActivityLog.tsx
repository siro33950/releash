import { invoke } from "@tauri-apps/api/core";
import {
	Check,
	ChevronRight,
	Copy,
	Layers,
	Loader2,
	Terminal,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ActivityEntry, MessagePart } from "@/types/session";
import { AgentEditPreviewPanel } from "./AgentEditPreviewPanel";
import type { TaskGroup } from "./toolPairing";

type ToolCategory = "read" | "write" | "command" | "other";

interface AgentToolActivityPresentation {
	category: ToolCategory;
	label: string;
	summary: string;
	editPreviewTool: boolean;
}

function truncateResult(
	content: string,
	maxLines = 5,
	maxChars = 4000,
): string {
	const lines = content.split("\n");
	const lineLimited =
		lines.length <= maxLines
			? content
			: `${lines.slice(0, maxLines).join("\n")}\n... (${lines.length - maxLines} more lines)`;
	if (lineLimited.length <= maxChars) return lineLimited;
	return `${lineLimited.slice(0, maxChars)}\n... (${lineLimited.length - maxChars} more chars)`;
}

export function CollapsibleError({
	content,
	maxLines = 5,
}: {
	content: string;
	maxLines?: number;
}) {
	const [isExpanded, setIsExpanded] = useState(false);
	return (
		<div className="py-0.5">
			<button
				type="button"
				className="flex items-center gap-1 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
				onClick={() => setIsExpanded(!isExpanded)}
			>
				<ChevronRight
					className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
				/>
				<span className="text-destructive">Error</span>
			</button>
			{isExpanded && (
				<CopyableToolResult content={content} maxLines={maxLines} />
			)}
		</div>
	);
}

function CopyableToolResult({
	content,
	maxLines = 5,
	className = "max-h-48",
}: {
	content: string;
	maxLines?: number;
	className?: string;
}) {
	const [copyState, setCopyState] = useState<"idle" | "copied" | "error">(
		"idle",
	);
	useEffect(() => {
		if (copyState === "idle") return;
		const timeout = window.setTimeout(() => setCopyState("idle"), 1600);
		return () => window.clearTimeout(timeout);
	}, [copyState]);
	const handleCopy = async () => {
		try {
			await navigator.clipboard.writeText(content);
			setCopyState("copied");
		} catch {
			setCopyState("error");
		}
	};

	return (
		<div className="relative mt-1 ml-4">
			<button
				type="button"
				className="absolute right-1 top-1 inline-flex size-6 items-center justify-center rounded text-muted-foreground/70 hover:bg-background hover:text-foreground"
				aria-label="Copy tool result"
				title={copyState === "error" ? "Copy failed" : "Copy tool result"}
				onClick={handleCopy}
			>
				{copyState === "copied" ? (
					<Check className="size-3.5" />
				) : (
					<Copy className="size-3.5" />
				)}
			</button>
			<pre
				className={`text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden overflow-y-auto pr-8 ${className}`}
			>
				{truncateResult(content, maxLines)}
			</pre>
		</div>
	);
}

function fallbackToolPresentation(
	tool: string,
	input: unknown,
): AgentToolActivityPresentation {
	void input;
	return {
		category: "other",
		label: tool,
		summary: tool,
		editPreviewTool: false,
	};
}

function useToolActivityPresentation(
	entry: Extract<ActivityEntry, { type: "tool_use" }>,
	basePath?: string,
): AgentToolActivityPresentation {
	const tool = entry.tool;
	const input = entry.input;
	const fallback = useMemo(
		() => fallbackToolPresentation(tool, input),
		[tool, input],
	);
	const [presentation, setPresentation] =
		useState<AgentToolActivityPresentation>(fallback);

	useEffect(() => {
		let cancelled = false;
		setPresentation(fallback);
		void invoke<AgentToolActivityPresentation>("present_agent_tool_activity", {
			toolName: entry.tool,
			input: entry.input,
			basePath,
		})
			.then((result) => {
				if (!cancelled) {
					setPresentation(result);
				}
			})
			.catch(() => {
				if (!cancelled) {
					setPresentation(fallback);
				}
			});
		return () => {
			cancelled = true;
		};
	}, [entry.tool, entry.input, basePath, fallback]);

	return presentation;
}

interface ToolActivityProps {
	entry: Extract<ActivityEntry, { type: "tool_use" }>;
	result?: Extract<ActivityEntry, { type: "tool_result" }>;
	index: number;
	isExecuting?: boolean;
	basePath?: string;
	presentation?: AgentToolActivityPresentation;
}

function ToolActivityHeader({
	index,
	isExecuting,
	isExpanded,
	onToggle,
	children,
	className,
}: {
	index: number;
	isExecuting?: boolean;
	isExpanded: boolean;
	onToggle: () => void;
	children: React.ReactNode;
	className?: string;
}) {
	return (
		<button
			type="button"
			data-testid={`activity-tool-use-${index}`}
			className={`flex items-center ${className ?? "gap-1"} min-w-0 text-muted-foreground/70 hover:text-foreground/80 transition-colors`}
			onClick={onToggle}
		>
			{isExecuting ? (
				<Loader2 className="size-3 shrink-0 animate-spin" />
			) : (
				<ChevronRight
					className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
				/>
			)}
			{children}
		</button>
	);
}

function ReadToolActivity({
	entry,
	result,
	index,
	isExecuting,
	presentation,
}: ToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const label = presentation?.label ?? entry.tool;

	return (
		<div className="py-0.5">
			<ToolActivityHeader
				index={index}
				isExecuting={isExecuting}
				isExpanded={isExpanded}
				onToggle={() => setIsExpanded(!isExpanded)}
			>
				<span className="truncate">{label}</span>
			</ToolActivityHeader>
			{result?.isError && result.content.trim().length > 0 ? (
				<CollapsibleError content={result.content} />
			) : isExpanded && result && result.content.trim().length > 0 ? (
				<CopyableToolResult content={result.content} className="max-h-48" />
			) : null}
		</div>
	);
}

function CommandToolActivity({
	entry,
	result,
	index,
	isExecuting,
	presentation,
}: ToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const label = presentation?.label ?? entry.tool;
	const hasResult = result && result.content.trim().length > 0;

	return (
		<div className="py-0.5">
			<ToolActivityHeader
				index={index}
				isExecuting={isExecuting}
				isExpanded={isExpanded}
				onToggle={() => setIsExpanded(!isExpanded)}
				className="gap-1.5"
			>
				<Terminal className="size-3 shrink-0" />
				<code className="truncate text-foreground/80">{label}</code>
			</ToolActivityHeader>
			{hasResult && result.isError ? (
				<CollapsibleError content={result.content} maxLines={20} />
			) : isExpanded && hasResult ? (
				<CopyableToolResult
					content={result.content}
					maxLines={20}
					className="max-h-64 rounded bg-muted px-2 py-1.5"
				/>
			) : null}
		</div>
	);
}

function DefaultToolActivity({
	entry,
	result,
	index,
	isExecuting,
	basePath,
	presentation,
}: ToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const hasInput = Object.keys(entry.input).length > 0;
	const summary = presentation?.summary ?? entry.tool;

	return (
		<div className="py-0.5">
			<ToolActivityHeader
				index={index}
				isExecuting={isExecuting}
				isExpanded={isExpanded}
				onToggle={() => setIsExpanded(!isExpanded)}
			>
				<span className="truncate">
					{entry.tool} {summary}
				</span>
			</ToolActivityHeader>
			{result?.isError && result.content.trim().length > 0 ? (
				<CollapsibleError content={result.content} />
			) : isExpanded ? (
				<>
					{presentation?.editPreviewTool && (
						<AgentEditPreviewPanel
							worktreePath={basePath}
							toolName={entry.tool}
							input={entry.input}
						/>
					)}
					{hasInput && (
						<pre className="mt-1 ml-4 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
							{JSON.stringify(entry.input, null, 2)}
						</pre>
					)}
					{result && result.content.trim().length > 0 && (
						<CopyableToolResult content={result.content} className="max-h-48" />
					)}
				</>
			) : null}
		</div>
	);
}

export function ToolActivity({
	entry,
	result,
	index,
	isExecuting,
	basePath,
}: ToolActivityProps) {
	const presentation = useToolActivityPresentation(entry, basePath);
	const category = presentation.category;

	switch (category) {
		case "read":
			return (
				<ReadToolActivity
					entry={entry}
					result={result}
					index={index}
					isExecuting={isExecuting}
					basePath={basePath}
					presentation={presentation}
				/>
			);
		case "command":
			return (
				<CommandToolActivity
					entry={entry}
					result={result}
					index={index}
					isExecuting={isExecuting}
					basePath={basePath}
					presentation={presentation}
				/>
			);
		default:
			return (
				<DefaultToolActivity
					entry={entry}
					result={result}
					index={index}
					isExecuting={isExecuting}
					basePath={basePath}
					presentation={presentation}
				/>
			);
	}
}

export function ActivityItem({
	entry,
	index,
}: {
	entry: ActivityEntry;
	index: number;
}) {
	const [isExpanded, setIsExpanded] = useState(false);

	if (entry.type === "permission_result") {
		return (
			<div
				data-testid={`activity-permission-result-${index}`}
				className="py-0.5"
			>
				<span className="text-muted-foreground/70">
					{entry.status === "allowed" ? "✓" : "✗"} {entry.toolName}:{" "}
					{entry.summary}
				</span>
			</div>
		);
	}

	if (entry.type === "tool_result") {
		const hasContent = entry.content.trim().length > 0;
		if (entry.isError && hasContent) {
			return (
				<div data-testid={`activity-tool-result-${index}`}>
					<CollapsibleError content={entry.content} />
				</div>
			);
		}
		const label = hasContent ? "✓" : "Done";
		return (
			<div data-testid={`activity-tool-result-${index}`} className="py-0.5">
				{hasContent ? (
					<button
						type="button"
						className="flex items-center gap-1 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
						onClick={() => setIsExpanded(!isExpanded)}
					>
						<ChevronRight
							className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
						/>
						<span>{label}</span>
					</button>
				) : (
					<span className="text-muted-foreground/70">{label}</span>
				)}
				{isExpanded && hasContent && (
					<CopyableToolResult content={entry.content} className="max-h-48" />
				)}
			</div>
		);
	}

	// tool_use without paired result — fallback
	return <ToolActivity entry={entry} index={index} />;
}

interface TaskToolActivityProps {
	group: TaskGroup;
	parts: MessagePart[];
	pairedResults: Map<number, Extract<MessagePart, { type: "tool_result" }>>;
	isStreaming: boolean;
	basePath?: string;
}

export function TaskToolActivity({
	group,
	parts,
	pairedResults,
	isStreaming,
	basePath,
}: TaskToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const isRunning = group.isBackground
		? !group.isCompleted
		: isStreaming && !group.isCompleted;

	const label = group.description
		? group.subagentType
			? `${group.description} (${group.subagentType})`
			: group.description
		: group.subagentType
			? `Task (${group.subagentType})`
			: "Task";

	return (
		<div className="py-0.5">
			<button
				type="button"
				data-testid={`activity-task-${group.toolUseIndex}`}
				className="flex items-center gap-1 min-w-0 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
				onClick={() => setIsExpanded(!isExpanded)}
			>
				{isRunning ? (
					<Loader2 className="size-3 shrink-0 animate-spin" />
				) : (
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
				)}
				<Layers className="size-3 shrink-0" />
				<span className="truncate">{label}</span>
				{group.isCompleted && (
					<span className="text-muted-foreground/50 ml-1">
						({group.childIndices.length} steps)
					</span>
				)}
			</button>
			{isExpanded && (
				<div className="ml-4 mt-0.5 border-l border-muted pl-2">
					{group.childIndices.map((ci) => {
						const child = parts[ci];
						if (!child) return null;
						const key = `task-child-${ci}`;
						switch (child.type) {
							case "text":
								return child.content.trim() ? (
									<div
										key={key}
										className="py-0.5 text-muted-foreground/70 text-[11px] whitespace-pre-wrap break-words max-h-24 overflow-hidden"
									>
										{child.content.length > 200
											? `${child.content.slice(0, 200)}…`
											: child.content}
									</div>
								) : null;
							case "tool_use": {
								const result = pairedResults.get(ci);
								const executing = isRunning && !result;
								return (
									<div key={key}>
										<ToolActivity
											entry={child}
											result={result}
											index={ci}
											isExecuting={executing}
											basePath={basePath}
										/>
									</div>
								);
							}
							case "error":
								return (
									<div key={key}>
										<CollapsibleError content={child.content} />
									</div>
								);
							default:
								return null;
						}
					})}
					{group.statusParts.map((sp) => {
						if (sp.status === "started" || sp.status === "progress")
							return null;
						return (
							<div
								key={`task-status-${group.toolUseId}-${sp.status}`}
								className="py-0.5 text-muted-foreground/50 text-[11px]"
							>
								{sp.status}
								{sp.summary ? `: ${sp.summary}` : ""}
							</div>
						);
					})}
				</div>
			)}
		</div>
	);
}
