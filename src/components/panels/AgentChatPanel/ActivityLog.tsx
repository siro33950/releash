import { invoke } from "@tauri-apps/api/core";
import {
	AlertCircle,
	Check,
	CheckCircle2,
	ChevronRight,
	Copy,
	FileText,
	Filter,
	Globe,
	Layers,
	Loader2,
	Moon,
	Pencil,
	Plug,
	Search,
	Square,
	Terminal,
	XCircle,
} from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ActivityEntry, MessagePart } from "@/types/session";
import { AgentEditPreviewPanel } from "./AgentEditPreviewPanel";
import type { TaskGroup } from "./toolPairing";

type ToolCategory = "read" | "write" | "command" | "mcp" | "other";

const READ_TOOL_NAMES = new Set([
	"Read",
	"Glob",
	"Grep",
	"WebFetch",
	"WebSearch",
	"ToolSearch",
	"ListMcpResourcesTool",
	"ReadMcpResourceTool",
]);

interface AgentToolActivityPresentation {
	category: ToolCategory;
	label: string;
	summary: string;
	editPreviewTool: boolean;
}

const toolPresentationCache = new Map<string, AgentToolActivityPresentation>();
const toolPresentationRequests = new Map<
	string,
	Promise<AgentToolActivityPresentation>
>();
const activityExpansionState = new Map<string, boolean>();
let scopedSessionId: string | null = null;

export function resetActivityLogUiStateForTest() {
	toolPresentationCache.clear();
	toolPresentationRequests.clear();
	activityExpansionState.clear();
	scopedSessionId = null;
}

function clearSessionScopedActivityLogUiState() {
	toolPresentationCache.clear();
	activityExpansionState.clear();
}

function syncActivityLogSessionScope(sessionId: string) {
	if (scopedSessionId === sessionId) return;
	if (scopedSessionId !== null) {
		clearSessionScopedActivityLogUiState();
	}
	scopedSessionId = sessionId;
}

export function syncActivityLogSessionScopeForTest(sessionId: string) {
	syncActivityLogSessionScope(sessionId);
}

export function useActivityLogSessionScope(sessionId: string) {
	useEffect(() => {
		syncActivityLogSessionScope(sessionId);
	}, [sessionId]);
}

function stableSerialize(value: unknown): string {
	if (value === null || typeof value !== "object") {
		return JSON.stringify(value) ?? String(value);
	}
	if (Array.isArray(value)) {
		return `[${value.map((item) => stableSerialize(item)).join(",")}]`;
	}
	return `{${Object.entries(value as Record<string, unknown>)
		.sort(([a], [b]) => a.localeCompare(b))
		.map(([key, item]) => `${JSON.stringify(key)}:${stableSerialize(item)}`)
		.join(",")}}`;
}

function samePresentation(
	a: AgentToolActivityPresentation,
	b: AgentToolActivityPresentation,
) {
	return (
		a.category === b.category &&
		a.label === b.label &&
		a.summary === b.summary &&
		a.editPreviewTool === b.editPreviewTool
	);
}

function usePersistentActivityExpansion(key: string) {
	const [isExpanded, setExpandedState] = useState(
		() => activityExpansionState.get(key) ?? false,
	);

	useEffect(() => {
		setExpandedState(activityExpansionState.get(key) ?? false);
	}, [key]);

	const setIsExpanded = useCallback(
		(next: boolean | ((current: boolean) => boolean)) => {
			setExpandedState((current) => {
				const resolved =
					typeof next === "function"
						? (next as (current: boolean) => boolean)(current)
						: next;
				activityExpansionState.set(key, resolved);
				return resolved;
			});
		},
		[key],
	);

	return [isExpanded, setIsExpanded] as const;
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

function copyableText(value: unknown): string {
	if (typeof value === "string") return value;
	return JSON.stringify(value, null, 2) ?? String(value);
}

function firstLine(content: string): { head: string; detail: string } {
	const trimmed = content.trim();
	const [head = "", ...rest] = trimmed.split("\n");
	return { head, detail: rest.join("\n").trim() };
}

function extractExitCode(content: string): string | null {
	const match = content.match(/exit code\s+(-?\d+)/i);
	return match?.[1] ?? null;
}

function readToolIcon(tool: string) {
	switch (tool) {
		case "Glob":
			return Filter;
		case "Grep":
		case "ToolSearch":
			return Search;
		case "WebFetch":
		case "WebSearch":
			return Globe;
		default:
			return FileText;
	}
}

function ToolStatusIcon({
	result,
}: {
	result?: Extract<ActivityEntry, { type: "tool_result" }>;
}) {
	if (!result) return null;
	if (result.isError) {
		return <XCircle className="size-3 shrink-0 text-destructive" />;
	}
	return <CheckCircle2 className="size-3 shrink-0 text-muted-foreground/70" />;
}

function SmallCopyButton({
	content,
	ariaLabel,
}: {
	content: string;
	ariaLabel: string;
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
		<button
			type="button"
			className="inline-flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground/70 hover:bg-muted hover:text-foreground"
			aria-label={ariaLabel}
			title={copyState === "error" ? "Copy failed" : ariaLabel}
			onClick={handleCopy}
		>
			{copyState === "copied" ? (
				<Check className="size-3.5" />
			) : (
				<Copy className="size-3.5" />
			)}
		</button>
	);
}

export function AgentErrorBlock({ content }: { content: string }) {
	const [isExpanded, setIsExpanded] = useState(false);
	const { head, detail } = firstLine(content);
	return (
		<div className="rounded border border-orange-500/60 px-3 py-2 text-orange-700 dark:text-orange-300">
			<div className="flex min-w-0 items-start gap-2">
				<AlertCircle className="mt-0.5 size-3.5 shrink-0" />
				<div className="min-w-0 flex-1">
					<div className="break-words">
						<span className="font-medium">Error:</span> {head || content}
					</div>
					{detail && (
						<>
							<button
								type="button"
								className="mt-1 inline-flex items-center gap-1 text-orange-700/80 hover:text-orange-700 dark:text-orange-300/80 dark:hover:text-orange-300"
								onClick={() => setIsExpanded((current) => !current)}
								aria-expanded={isExpanded}
							>
								<ChevronRight
									className={`size-3 transition-transform ${isExpanded ? "rotate-90" : ""}`}
								/>
								Details
							</button>
							{isExpanded && (
								<pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words text-[11px]">
									{detail}
								</pre>
							)}
						</>
					)}
				</div>
			</div>
		</div>
	);
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
				className="flex min-w-0 items-center gap-1 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
				onClick={() => setIsExpanded(!isExpanded)}
			>
				<ChevronRight
					className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
				/>
				<XCircle className="size-3 shrink-0 text-destructive" />
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
	terminal = false,
}: {
	content: string;
	maxLines?: number;
	className?: string;
	terminal?: boolean;
}) {
	return (
		<div className="relative mt-1 ml-4">
			<div className="absolute right-1 top-1">
				<SmallCopyButton content={content} ariaLabel="Copy tool result" />
			</div>
			<pre
				className={`whitespace-pre-wrap break-words overflow-hidden overflow-y-auto pr-8 ${
					terminal
						? "rounded bg-zinc-950 px-2 py-1.5 font-mono text-[11px] text-zinc-100"
						: "text-[11px] text-muted-foreground/70"
				} ${className}`}
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
	const inputRecord =
		input && typeof input === "object" && !Array.isArray(input)
			? (input as Record<string, unknown>)
			: {};
	const category: ToolCategory = tool.startsWith("mcp__")
		? "mcp"
		: READ_TOOL_NAMES.has(tool)
			? "read"
			: tool === "Bash"
				? "command"
				: ["Write", "Edit", "MultiEdit", "NotebookEdit"].includes(tool)
					? "write"
					: "other";
	const firstString = Object.values(inputRecord).find(
		(value): value is string => typeof value === "string",
	);
	const summary =
		typeof inputRecord.file_path === "string"
			? inputRecord.file_path
			: typeof inputRecord.command === "string"
				? inputRecord.command
				: (firstString ?? tool);
	return {
		category,
		label: summary === tool ? tool : `${tool} ${summary}`,
		summary,
		editPreviewTool: ["Edit", "MultiEdit", "Write"].includes(tool),
	};
}

export function fallbackToolPresentationForTest(
	tool: string,
	input: unknown,
): AgentToolActivityPresentation {
	return fallbackToolPresentation(tool, input);
}

function useToolActivityPresentation(
	entry: Extract<ActivityEntry, { type: "tool_use" }>,
	basePath?: string,
): AgentToolActivityPresentation {
	const tool = entry.tool;
	const input = entry.input;
	const inputRef = useRef(input);
	inputRef.current = input;
	const inputKey = useMemo(() => stableSerialize(input), [input]);
	const cacheKey = useMemo(
		() => `${tool}\u0000${basePath ?? ""}\u0000${inputKey}`,
		[tool, basePath, inputKey],
	);
	const fallback = fallbackToolPresentation(tool, input);
	const [presentationState, setPresentationState] = useState(() => ({
		key: cacheKey,
		presentation: toolPresentationCache.get(cacheKey) ?? fallback,
	}));

	useEffect(() => {
		let cancelled = false;
		const fallbackForKey = fallbackToolPresentation(tool, inputRef.current);

		const cached = toolPresentationCache.get(cacheKey);
		if (cached) {
			setPresentationState((current) =>
				current.key === cacheKey &&
				samePresentation(current.presentation, cached)
					? current
					: { key: cacheKey, presentation: cached },
			);
			return () => {
				cancelled = true;
			};
		}

		setPresentationState((current) =>
			current.key === cacheKey
				? current
				: { key: cacheKey, presentation: fallbackForKey },
		);

		let request = toolPresentationRequests.get(cacheKey);
		if (!request) {
			request = invoke<AgentToolActivityPresentation>(
				"present_agent_tool_activity",
				{
					toolName: tool,
					input: inputRef.current,
					basePath,
				},
			)
				.then((result) => {
					toolPresentationCache.set(cacheKey, result);
					return result;
				})
				.finally(() => {
					toolPresentationRequests.delete(cacheKey);
				});
			toolPresentationRequests.set(cacheKey, request);
		}

		void request
			.then((result) => {
				if (!cancelled) {
					setPresentationState((current) =>
						current.key === cacheKey &&
						samePresentation(current.presentation, result)
							? current
							: { key: cacheKey, presentation: result },
					);
				}
			})
			.catch(() => {
				if (!cancelled) {
					setPresentationState((current) =>
						current.key === cacheKey &&
						samePresentation(current.presentation, fallbackForKey)
							? current
							: { key: cacheKey, presentation: fallbackForKey },
					);
				}
			});
		return () => {
			cancelled = true;
		};
	}, [cacheKey, tool, basePath]);

	return presentationState.key === cacheKey
		? presentationState.presentation
		: (toolPresentationCache.get(cacheKey) ?? fallback);
}

interface ToolActivityProps {
	entry: Extract<ActivityEntry, { type: "tool_use" }>;
	result?: Extract<ActivityEntry, { type: "tool_result" }>;
	index: number;
	isExecuting?: boolean;
	basePath?: string;
	presentation?: AgentToolActivityPresentation;
	onOpenDiffFile?: (filePath: string) => void;
}

function ToolActivityHeader({
	index,
	isExpanded,
	onToggle,
	icon: Icon,
	label,
	meta,
	result,
	isExecuting,
}: {
	index: number;
	isExpanded: boolean;
	onToggle: () => void;
	icon: typeof FileText;
	label: React.ReactNode;
	meta?: React.ReactNode;
	result?: Extract<ActivityEntry, { type: "tool_result" }>;
	isExecuting?: boolean;
}) {
	return (
		<button
			type="button"
			data-testid={`activity-tool-use-${index}`}
			className="flex w-full min-w-0 max-w-full items-center gap-1.5 text-left text-muted-foreground/75 transition-colors hover:text-foreground/85"
			onClick={onToggle}
			title={typeof label === "string" ? label : undefined}
		>
			{isExecuting ? (
				<Loader2 className="size-3 shrink-0 animate-spin" />
			) : (
				<ChevronRight
					className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
				/>
			)}
			<Icon className="size-3 shrink-0" />
			<span className="min-w-0 flex-1 truncate text-left">{label}</span>
			{meta && (
				<span className="shrink-0 text-muted-foreground/50">{meta}</span>
			)}
			<ToolStatusIcon result={result} />
		</button>
	);
}

export function ToolActivity({
	entry,
	result,
	index,
	isExecuting,
	basePath,
	onOpenDiffFile,
}: ToolActivityProps) {
	const presentation = useToolActivityPresentation(entry, basePath);
	const category = presentation.category;
	const expansionKey = `tool:${entry.id || `${index}:${entry.tool}:${stableSerialize(entry.input)}`}`;
	const [isExpanded, setIsExpanded] =
		usePersistentActivityExpansion(expansionKey);
	const hasResult = result && result.content.trim().length > 0;
	const hasInput = Object.keys(entry.input).length > 0;
	const isCommand = category === "command";
	const isWrite = category === "write";
	const isMcp = category === "mcp";
	const showInputJson = hasInput && (!isWrite || !basePath);
	const args = isMcp
		? "arguments" in entry.input
			? entry.input.arguments
			: entry.input
		: entry.input;
	const Icon =
		category === "read"
			? readToolIcon(entry.tool)
			: isCommand
				? Terminal
				: isWrite
					? Pencil
					: isMcp
						? Plug
						: FileText;
	const exitCode =
		hasResult && isCommand ? extractExitCode(result.content) : null;
	const meta =
		isCommand && exitCode !== null ? (
			<span className="rounded border border-border px-1">exit {exitCode}</span>
		) : undefined;

	return (
		<div className="py-0.5">
			<ToolActivityHeader
				index={index}
				isExecuting={isExecuting}
				isExpanded={isExpanded}
				onToggle={() => setIsExpanded(!isExpanded)}
				icon={Icon}
				label={presentation.label}
				meta={meta}
				result={result}
			/>
			{result?.isError && hasResult ? (
				<CollapsibleError
					content={result.content}
					maxLines={isCommand ? 20 : 5}
				/>
			) : isExpanded ? (
				<>
					{isCommand && (
						<div className="mt-1 ml-4 flex min-w-0 items-center gap-1 rounded bg-muted/30 px-2 py-1 font-mono text-[11px] text-muted-foreground">
							<code className="min-w-0 flex-1 truncate">
								{presentation.label}
							</code>
							<SmallCopyButton
								content={presentation.label}
								ariaLabel="Copy command"
							/>
						</div>
					)}
					{isWrite && presentation.editPreviewTool && (
						<AgentEditPreviewPanel
							worktreePath={basePath}
							toolName={entry.tool}
							input={entry.input}
							onOpenDiffFile={onOpenDiffFile}
						/>
					)}
					{isMcp && (
						<pre className="mt-1 ml-4 max-h-40 overflow-auto whitespace-pre-wrap break-words text-[11px] text-muted-foreground/70">
							{copyableText(args)}
						</pre>
					)}
					{showInputJson && !isMcp && (
						<pre className="mt-1 ml-4 max-h-48 overflow-y-auto whitespace-pre-wrap break-words text-[11px] text-muted-foreground/70">
							{JSON.stringify(entry.input, null, 2)}
						</pre>
					)}
					{hasResult && (
						<CopyableToolResult
							content={result.content}
							maxLines={isCommand ? 20 : 5}
							className={isCommand ? "max-h-64" : "max-h-48"}
							terminal={isCommand}
						/>
					)}
				</>
			) : null}
		</div>
	);
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
						<CheckCircle2 className="size-3 shrink-0" />
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

	return <ToolActivity entry={entry} index={index} />;
}

interface TaskToolActivityProps {
	group: TaskGroup;
	parts: MessagePart[];
	pairedResults: Map<number, Extract<MessagePart, { type: "tool_result" }>>;
	isStreaming: boolean;
	basePath?: string;
	onOpenDiffFile?: (filePath: string) => void;
}

function taskStatus(group: TaskGroup, isRunning: boolean) {
	const latest = group.statusParts[group.statusParts.length - 1];
	if (isRunning) return "running";
	if (latest?.status === "backgrounded") return "backgrounded";
	if (latest?.status === "failed") return "failed";
	if (latest?.status === "stopped") return "stopped";
	if (group.isCompleted) return "completed";
	return latest?.status ?? "pending";
}

function TaskStatusIcon({ status }: { status: string }) {
	switch (status) {
		case "running":
		case "progress":
			return <Loader2 className="size-3 shrink-0 animate-spin" />;
		case "completed":
			return <CheckCircle2 className="size-3 shrink-0" />;
		case "failed":
			return <AlertCircle className="size-3 shrink-0 text-destructive" />;
		case "backgrounded":
			return <Moon className="size-3 shrink-0" />;
		case "stopped":
			return <Square className="size-3 shrink-0" />;
		default:
			return <Layers className="size-3 shrink-0" />;
	}
}

function taskStatusClass(status: string) {
	switch (status) {
		case "failed":
			return "text-destructive";
		case "backgrounded":
			return "text-muted-foreground";
		case "completed":
			return "text-muted-foreground/70";
		default:
			return "text-foreground/80";
	}
}

export function TaskToolActivity({
	group,
	parts,
	pairedResults,
	isStreaming,
	basePath,
	onOpenDiffFile,
}: TaskToolActivityProps) {
	const [isExpanded, setIsExpanded] = usePersistentActivityExpansion(
		`task:${group.toolUseId}`,
	);
	const isRunning = group.isBackground
		? !group.isCompleted
		: isStreaming && !group.isCompleted;
	const status = taskStatus(group, isRunning);
	const latestSummary = [...group.statusParts]
		.reverse()
		.find((part) => part.summary)?.summary;

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
				className={`flex w-full min-w-0 max-w-full items-center gap-1.5 text-left transition-colors hover:text-foreground/85 ${taskStatusClass(status)}`}
				onClick={() => setIsExpanded(!isExpanded)}
				title={label}
			>
				{isRunning ? (
					<Loader2 className="size-3 shrink-0 animate-spin" />
				) : (
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
				)}
				<TaskStatusIcon status={status} />
				<span className="min-w-0 flex-1 truncate text-left">{label}</span>
				{group.isCompleted && (
					<span className="text-muted-foreground/50">
						({group.childIndices.length} steps)
					</span>
				)}
			</button>
			{latestSummary && (
				<div className="ml-7 mt-0.5 truncate text-[11px] text-muted-foreground/70">
					{latestSummary}
				</div>
			)}
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
											? `${child.content.slice(0, 200)}...`
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
											onOpenDiffFile={onOpenDiffFile}
										/>
									</div>
								);
							}
							case "error":
								return (
									<div key={key} className="py-0.5">
										<AgentErrorBlock content={child.content} />
									</div>
								);
							default:
								return null;
						}
					})}
				</div>
			)}
		</div>
	);
}
