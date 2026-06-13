import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type PreviewLineKind = "context" | "removed" | "added";

interface AgentEditPreviewLine {
	kind: PreviewLineKind;
	oldLine?: number | null;
	newLine?: number | null;
	content: string;
}

interface AgentEditPreviewHunk {
	oldStart: number;
	newStart: number;
	lines: AgentEditPreviewLine[];
}

interface AgentEditPreview {
	toolName: string;
	operation: string;
	filePath?: string | null;
	hunks: AgentEditPreviewHunk[];
	warnings: string[];
}

interface AgentEditPreviewPanelProps {
	worktreePath?: string;
	toolName: string;
	input: Record<string, unknown>;
}

function isPreview(value: unknown): value is AgentEditPreview {
	if (!value || typeof value !== "object") return false;
	const candidate = value as Partial<AgentEditPreview>;
	return (
		typeof candidate.toolName === "string" &&
		typeof candidate.operation === "string" &&
		Array.isArray(candidate.hunks) &&
		Array.isArray(candidate.warnings)
	);
}

function lineClass(kind: PreviewLineKind): string {
	switch (kind) {
		case "added":
			return "bg-[color-mix(in_oklch,var(--status-added)_14%,transparent)]";
		case "removed":
			return "bg-[color-mix(in_oklch,var(--status-deleted)_14%,transparent)]";
		default:
			return "";
	}
}

function lineMarker(kind: PreviewLineKind): string {
	switch (kind) {
		case "added":
			return "+";
		case "removed":
			return "-";
		default:
			return " ";
	}
}

export function AgentEditPreviewPanel({
	worktreePath,
	toolName,
	input,
}: AgentEditPreviewPanelProps) {
	const [preview, setPreview] = useState<AgentEditPreview | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!worktreePath) {
			setPreview(null);
			setError(null);
			return;
		}

		let canceled = false;
		invoke<unknown>("build_agent_edit_preview", {
			worktreePath,
			toolName,
			input,
		})
			.then((value) => {
				if (canceled) return;
				setPreview(isPreview(value) ? value : null);
				setError(null);
			})
			.catch((e) => {
				if (canceled) return;
				setPreview(null);
				setError(String(e));
			});

		return () => {
			canceled = true;
		};
	}, [worktreePath, toolName, input]);

	if (!worktreePath || (!preview && !error)) return null;

	return (
		<div className="mt-1 ml-4 overflow-hidden rounded border border-border bg-background/60 text-[11px]">
			<div className="flex min-w-0 items-center justify-between gap-2 border-b border-border px-2 py-1 text-muted-foreground">
				<span className="min-w-0 truncate">
					{preview?.operation ?? "Edit preview unavailable"}
					{preview?.filePath ? `: ${preview.filePath}` : ""}
				</span>
			</div>
			{error && (
				<div className="px-2 py-1 text-destructive">
					Preview unavailable: {error}
				</div>
			)}
			{preview?.warnings.map((warning) => (
				<div key={warning} className="px-2 py-1 text-muted-foreground">
					{warning}
				</div>
			))}
			{preview &&
				preview.hunks.length === 0 &&
				preview.warnings.length === 0 && (
					<div className="px-2 py-1 text-muted-foreground">
						No text changes detected.
					</div>
				)}
			{preview?.hunks.map((hunk, hunkIndex) => (
				<div
					// biome-ignore lint/suspicious/noArrayIndexKey: hunk order is stable from Rust diff output
					key={`${hunk.oldStart}:${hunk.newStart}:${hunkIndex}`}
					className="overflow-x-auto"
				>
					<div className="border-b border-border/60 px-2 py-0.5 font-mono text-muted-foreground">
						@@ -{hunk.oldStart} +{hunk.newStart} @@
					</div>
					{hunk.lines.map((line, lineIndex) => (
						<div
							// biome-ignore lint/suspicious/noArrayIndexKey: diff line order is stable from Rust diff output
							key={`${line.kind}:${line.oldLine ?? ""}:${line.newLine ?? ""}:${lineIndex}`}
							className={`grid grid-cols-[3ch_3ch_2ch_minmax(0,1fr)] gap-1 px-2 font-mono ${lineClass(line.kind)}`}
						>
							<span className="select-none text-right text-muted-foreground/60">
								{line.oldLine ?? ""}
							</span>
							<span className="select-none text-right text-muted-foreground/60">
								{line.newLine ?? ""}
							</span>
							<span className="select-none text-muted-foreground">
								{lineMarker(line.kind)}
							</span>
							<span className="whitespace-pre">{line.content || " "}</span>
						</div>
					))}
				</div>
			))}
		</div>
	);
}
