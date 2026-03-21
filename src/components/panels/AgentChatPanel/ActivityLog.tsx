import { ChevronRight, Loader2 } from "lucide-react";
import { useState } from "react";
import type { ActivityEntry } from "@/types/session";

interface ActivityLogProps {
	activities: ActivityEntry[];
	isStreaming: boolean;
}

function summarizeToolInput(
	tool: string,
	input: Record<string, unknown>,
): string {
	if (input.file_path && typeof input.file_path === "string") {
		return input.file_path;
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

function truncateResult(content: string, maxLines = 5): string {
	const lines = content.split("\n");
	if (lines.length <= maxLines) return content;
	return `${lines.slice(0, maxLines).join("\n")}\n… (${lines.length - maxLines} more lines)`;
}

export function ActivityItem({
	entry,
	index,
}: {
	entry: ActivityEntry;
	index: number;
}) {
	const [isExpanded, setIsExpanded] = useState(false);

	if (entry.type === "tool_use") {
		return (
			<div data-testid={`activity-tool-use-${index}`} className="py-0.5">
				<span className="font-medium text-foreground/80">{entry.tool}</span>
				<span className="ml-1.5 text-muted-foreground/70">
					{summarizeToolInput(entry.tool, entry.input)}
				</span>
			</div>
		);
	}

	if (entry.type === "permission_result") {
		return (
			<div data-testid={`activity-permission-result-${index}`} className="py-0.5">
				<span className="text-muted-foreground/70">
					{entry.status === "allowed" ? "✓" : "✗"} {entry.toolName}: {entry.summary}
				</span>
			</div>
		);
	}

	const hasContent = entry.content.trim().length > 0;

	return (
		<div data-testid={`activity-tool-result-${index}`} className="py-0.5">
			{entry.isError ? (
				<span className="text-destructive">Error</span>
			) : (
				<span className="text-muted-foreground/70">
					{hasContent ? "✓" : "Done"}
				</span>
			)}
			{hasContent && (
				<>
					<button
						type="button"
						className="ml-1.5 text-muted-foreground/50 hover:text-foreground/70 transition-colors"
						onClick={() => setIsExpanded(!isExpanded)}
					>
						{isExpanded ? "hide" : "show"}
					</button>
					{isExpanded && (
						<pre className="mt-1 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
							{truncateResult(entry.content)}
						</pre>
					)}
				</>
			)}
		</div>
	);
}

export function ActivityLog({ activities, isStreaming }: ActivityLogProps) {
	const [isOpen, setIsOpen] = useState(false);

	if (activities.length === 0) return null;

	const toolUseCount = activities.filter((a) => a.type === "tool_use").length;

	return (
		<div data-testid="activity-log" className="px-4 py-2">
			<button
				type="button"
				data-testid="activity-log-toggle"
				className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
				onClick={() => setIsOpen(!isOpen)}
			>
				<ChevronRight
					className={`size-3 transition-transform ${isOpen ? "rotate-90" : ""}`}
				/>
				<span>
					{toolUseCount} tool {toolUseCount === 1 ? "call" : "calls"}
				</span>
				{isStreaming && <Loader2 className="size-3 animate-spin ml-1" />}
			</button>
			{isOpen && (
				<div className="mt-1 pl-4 text-xs border-l border-border/50 space-y-0">
					{activities.map((entry, idx) => {
						const key = entry.type === "tool_use" ? entry.id : `result-${idx}`;
						return <ActivityItem key={key} entry={entry} index={idx} />;
					})}
				</div>
			)}
		</div>
	);
}
