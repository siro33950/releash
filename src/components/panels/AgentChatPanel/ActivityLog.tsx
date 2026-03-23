import { ChevronRight, Loader2, Terminal } from "lucide-react";
import { useState } from "react";
import type { ActivityEntry } from "@/types/session";
import {
	classifyTool,
	getCommandLabel,
	getReadToolLabel,
} from "./toolClassification";

function truncateResult(content: string, maxLines = 5): string {
	const lines = content.split("\n");
	if (lines.length <= maxLines) return content;
	return `${lines.slice(0, maxLines).join("\n")}\n… (${lines.length - maxLines} more lines)`;
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
				<pre className="mt-1 ml-4 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
					{truncateResult(content, maxLines)}
				</pre>
			)}
		</div>
	);
}

function shortenPath(fullPath: string, basePath?: string): string {
	if (basePath && fullPath.startsWith(basePath)) {
		const rel = fullPath.slice(basePath.length);
		return rel.startsWith("/") ? rel.slice(1) : rel;
	}
	return fullPath;
}

function summarizeToolInput(
	tool: string,
	input: Record<string, unknown>,
	basePath?: string,
): string {
	if (input.file_path && typeof input.file_path === "string") {
		return shortenPath(input.file_path, basePath);
	}
	if (input.pattern && typeof input.pattern === "string") {
		return input.pattern;
	}
	if (input.command && typeof input.command === "string") {
		const cmd = input.command as string;
		return cmd.length > 80 ? `${cmd.slice(0, 80)}…` : cmd;
	}
	const keys = Object.keys(input);
	if (keys.length === 0) return tool;
	const firstKey = keys[0];
	const val = input[firstKey];
	if (typeof val === "string") {
		return val.length > 60 ? `${val.slice(0, 60)}…` : val;
	}
	return `${firstKey}: …`;
}

interface ToolActivityProps {
	entry: Extract<ActivityEntry, { type: "tool_use" }>;
	result?: Extract<ActivityEntry, { type: "tool_result" }>;
	index: number;
	isExecuting?: boolean;
	basePath?: string;
}

function ReadToolActivity({
	entry,
	result,
	index,
	isExecuting,
	basePath,
}: ToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const label = getReadToolLabel(entry.tool, entry.input, basePath);

	return (
		<div data-testid={`activity-tool-use-${index}`} className="py-0.5">
			<button
				type="button"
				className="flex items-center gap-1 min-w-0 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
				onClick={() => setIsExpanded(!isExpanded)}
			>
				{isExecuting ? (
					<Loader2 className="size-3 shrink-0 animate-spin" />
				) : (
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
				)}
				<span className="truncate">{label}</span>
			</button>
			{isExpanded && result && result.content.trim().length > 0 && (
				<pre className="mt-1 ml-4 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
					{truncateResult(result.content)}
				</pre>
			)}
		</div>
	);
}

function CommandToolActivity({
	entry,
	result,
	index,
	isExecuting,
}: ToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const label = getCommandLabel(entry.input);
	const hasResult = result && result.content.trim().length > 0;

	return (
		<div data-testid={`activity-tool-use-${index}`} className="py-0.5">
			<button
				type="button"
				className="flex items-center gap-1.5 min-w-0 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
				onClick={() => setIsExpanded(!isExpanded)}
			>
				{isExecuting ? (
					<Loader2 className="size-3 shrink-0 animate-spin" />
				) : (
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
				)}
				<Terminal className="size-3 shrink-0" />
				<code className="truncate text-foreground/80">{label}</code>
			</button>
			{hasResult && result.isError ? (
				<CollapsibleError content={result.content} maxLines={20} />
			) : isExpanded && hasResult ? (
				<pre className="mt-1 ml-4 text-[11px] whitespace-pre-wrap break-words overflow-hidden max-h-64 overflow-y-auto rounded px-2 py-1.5 bg-muted text-muted-foreground/70">
					{truncateResult(result.content, 20)}
				</pre>
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
}: ToolActivityProps) {
	const [isExpanded, setIsExpanded] = useState(false);
	const hasInput = Object.keys(entry.input).length > 0;
	const summary = summarizeToolInput(entry.tool, entry.input, basePath);

	return (
		<div data-testid={`activity-tool-use-${index}`} className="py-0.5">
			<button
				type="button"
				className="flex items-center gap-1 min-w-0 text-muted-foreground/70 hover:text-foreground/80 transition-colors"
				onClick={() => setIsExpanded(!isExpanded)}
			>
				{isExecuting ? (
					<Loader2 className="size-3 shrink-0 animate-spin" />
				) : (
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
				)}
				<span className="truncate">
					{entry.tool} {summary}
				</span>
			</button>
			{isExpanded && (
				<>
					{hasInput && (
						<pre className="mt-1 ml-4 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
							{JSON.stringify(entry.input, null, 2)}
						</pre>
					)}
					{result && result.content.trim().length > 0 && (
						<pre className="mt-1 ml-4 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
							{truncateResult(result.content)}
						</pre>
					)}
				</>
			)}
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
	const category = classifyTool(entry.tool);

	switch (category) {
		case "read":
			return (
				<ReadToolActivity
					entry={entry}
					result={result}
					index={index}
					isExecuting={isExecuting}
					basePath={basePath}
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
					<pre className="mt-1 ml-4 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
						{truncateResult(entry.content)}
					</pre>
				)}
			</div>
		);
	}

	// tool_use without paired result — fallback
	return <ToolActivity entry={entry} index={index} />;
}
