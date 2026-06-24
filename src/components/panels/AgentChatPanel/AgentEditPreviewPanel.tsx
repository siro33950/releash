import { invoke } from "@tauri-apps/api/core";
import {
	Check,
	ChevronRight,
	ExternalLink,
	FileDiff,
	GitCompare,
	Plus,
	SquarePen,
	Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Hunk } from "@/lib/computeHunks";
import { DiffViewerSection } from "../DiffViewerSection";

interface AgentEditPreviewLine {
	kind: "context" | "removed" | "added";
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
	originalContent: string;
	modifiedContent: string;
	hunks: AgentEditPreviewHunk[];
	warnings: string[];
}

interface AgentEditPreviewPanelProps {
	worktreePath?: string;
	toolName: string;
	input: Record<string, unknown>;
	onOpenDiffFile?: (filePath: string) => void;
}

interface AgentEditPreviewState {
	preview: AgentEditPreview | null;
	error: string | null;
}

const editPreviewCache = new Map<string, AgentEditPreviewState>();
const editPreviewRequests = new Map<string, Promise<AgentEditPreviewState>>();
const editPreviewExpansionState = new Map<string, boolean>();

export function resetAgentEditPreviewPanelStateForTest() {
	editPreviewCache.clear();
	editPreviewRequests.clear();
	editPreviewExpansionState.clear();
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

function usePersistentPreviewExpansion(key: string, defaultExpanded = false) {
	const [isExpanded, setExpandedState] = useState(
		() => editPreviewExpansionState.get(key) ?? defaultExpanded,
	);

	useEffect(() => {
		setExpandedState(editPreviewExpansionState.get(key) ?? defaultExpanded);
	}, [key, defaultExpanded]);

	const setIsExpanded = useCallback(
		(next: boolean | ((current: boolean) => boolean)) => {
			setExpandedState((current) => {
				const resolved =
					typeof next === "function"
						? (next as (current: boolean) => boolean)(current)
						: next;
				editPreviewExpansionState.set(key, resolved);
				return resolved;
			});
		},
		[key],
	);

	return [isExpanded, setIsExpanded] as const;
}

function isPreview(value: unknown): value is AgentEditPreview {
	if (!value || typeof value !== "object") return false;
	const candidate = value as Partial<AgentEditPreview>;
	return (
		typeof candidate.toolName === "string" &&
		typeof candidate.operation === "string" &&
		typeof candidate.originalContent === "string" &&
		typeof candidate.modifiedContent === "string" &&
		Array.isArray(candidate.hunks) &&
		Array.isArray(candidate.warnings)
	);
}

function countChangedLines(preview: AgentEditPreview | null) {
	if (!preview) return { added: 0, removed: 0 };
	let added = 0;
	let removed = 0;
	for (const hunk of preview.hunks) {
		for (const line of hunk.lines) {
			if (line.kind === "added") added += 1;
			if (line.kind === "removed") removed += 1;
		}
	}
	return { added, removed };
}

function previewHunksToDiffHunks(hunks: AgentEditPreviewHunk[]): Hunk[] {
	return hunks.map((hunk, index) => {
		let oldLines = 0;
		let newLines = 0;
		const lines = hunk.lines.map((line) => {
			if (line.kind === "removed") {
				oldLines += 1;
				return `-${line.content}`;
			}
			if (line.kind === "added") {
				newLines += 1;
				return `+${line.content}`;
			}
			oldLines += 1;
			newLines += 1;
			return ` ${line.content}`;
		});

		return {
			index,
			oldStart: hunk.oldStart,
			oldLines,
			newStart: hunk.newStart,
			newLines,
			lines,
		};
	});
}

function changeKind(preview: AgentEditPreview | null): "A" | "M" | "D" | null {
	if (!preview) return null;
	const operation = preview.operation.toLowerCase();
	if (
		operation.includes("delete") ||
		operation.includes("remove") ||
		(preview.originalContent.length > 0 && preview.modifiedContent.length === 0)
	) {
		return "D";
	}
	if (
		operation.includes("create") ||
		operation.includes("add") ||
		operation.includes("write") ||
		(preview.originalContent.length === 0 && preview.modifiedContent.length > 0)
	) {
		return "A";
	}
	return "M";
}

function ChangeKindBadge({ kind }: { kind: "A" | "M" | "D" }) {
	const Icon = kind === "A" ? Plus : kind === "D" ? Trash2 : SquarePen;
	const label = kind === "A" ? "Added" : kind === "D" ? "Deleted" : "Modified";
	return (
		<span
			className="inline-flex h-5 shrink-0 items-center gap-1 rounded border border-border bg-muted/50 px-1.5 text-[10px] font-medium text-muted-foreground"
			title={label}
		>
			<Icon className="size-3" />
			{kind}
		</span>
	);
}

function absoluteFilePath(worktreePath: string | undefined, filePath: string) {
	if (filePath.startsWith("/") || !worktreePath) return filePath;
	return `${worktreePath}/${filePath}`;
}

function diffFilePath(worktreePath: string | undefined, filePath: string) {
	if (!worktreePath || !filePath.startsWith(`${worktreePath}/`)) {
		return filePath;
	}
	return filePath.slice(worktreePath.length + 1);
}

export function AgentEditPreviewPanel({
	worktreePath,
	toolName,
	input,
	onOpenDiffFile,
}: AgentEditPreviewPanelProps) {
	const inputRef = useRef(input);
	inputRef.current = input;
	const inputKey = useMemo(() => stableSerialize(input), [input]);
	const previewKey = useMemo(
		() => `${worktreePath ?? ""}\u0000${toolName}\u0000${inputKey}`,
		[worktreePath, toolName, inputKey],
	);
	const [previewState, setPreviewState] = useState<{
		key: string;
		value: AgentEditPreviewState;
	}>(() => ({
		key: previewKey,
		value: editPreviewCache.get(previewKey) ?? {
			preview: null,
			error: null,
		},
	}));
	const [isExpanded, setIsExpanded] = usePersistentPreviewExpansion(
		previewKey,
		true,
	);
	const [openState, setOpenState] = useState<"idle" | "opened" | "error">(
		"idle",
	);

	useEffect(() => {
		if (!worktreePath) {
			setPreviewState({
				key: previewKey,
				value: { preview: null, error: null },
			});
			return;
		}

		let canceled = false;

		const cached = editPreviewCache.get(previewKey);
		if (cached) {
			setPreviewState({ key: previewKey, value: cached });
			return () => {
				canceled = true;
			};
		}

		setPreviewState((current) =>
			current.key === previewKey
				? current
				: { key: previewKey, value: { preview: null, error: null } },
		);

		let request = editPreviewRequests.get(previewKey);
		if (!request) {
			request = invoke<unknown>("build_agent_edit_preview", {
				worktreePath,
				toolName,
				input: inputRef.current,
			})
				.then((value) => ({
					preview: isPreview(value) ? value : null,
					error: null,
				}))
				.catch(
					(e): AgentEditPreviewState => ({
						preview: null,
						error: String(e),
					}),
				)
				.then((state) => {
					editPreviewCache.set(previewKey, state);
					return state;
				})
				.finally(() => {
					editPreviewRequests.delete(previewKey);
				});
			editPreviewRequests.set(previewKey, request);
		}

		void request.then((state) => {
			if (canceled) return;
			setPreviewState({ key: previewKey, value: state });
		});

		return () => {
			canceled = true;
		};
	}, [worktreePath, toolName, previewKey]);

	useEffect(() => {
		if (openState === "idle") return;
		const timeout = window.setTimeout(() => setOpenState("idle"), 1600);
		return () => window.clearTimeout(timeout);
	}, [openState]);

	const { preview, error } =
		previewState.key === previewKey
			? previewState.value
			: (editPreviewCache.get(previewKey) ?? {
					preview: null,
					error: null,
				});
	const changedLines = useMemo(() => countChangedLines(preview), [preview]);
	const diffHunks = useMemo(
		() => (preview ? previewHunksToDiffHunks(preview.hunks) : []),
		[preview],
	);
	const kind = useMemo(() => changeKind(preview), [preview]);
	const canExpand =
		Boolean(error) ||
		Boolean(
			preview && (preview.hunks.length > 0 || preview.warnings.length > 0),
		);
	const filePath = preview?.filePath ?? null;
	const handleOpenInEditor = () => {
		if (!filePath) return;
		setOpenState("idle");
		invoke("open_in_editor", {
			filePath: absoluteFilePath(worktreePath, filePath),
		})
			.then(() => setOpenState("opened"))
			.catch(() => setOpenState("error"));
	};

	if (!worktreePath) return null;

	if (!preview && !error) {
		return (
			<div className="mt-1 ml-4 overflow-hidden rounded border border-border bg-background text-[11px]">
				<div className="flex min-w-0 w-full items-center gap-2 px-2 py-1 text-left text-muted-foreground">
					<FileDiff className="size-3 shrink-0" />
					<span className="min-w-0 flex-1 truncate">Loading edit diff...</span>
				</div>
			</div>
		);
	}

	return (
		<div className="mt-1 ml-4 overflow-hidden rounded border border-border bg-background text-[11px]">
			<button
				type="button"
				className="flex min-w-0 w-full items-center gap-2 px-2 py-1 text-left text-muted-foreground hover:text-foreground"
				aria-expanded={isExpanded}
				onClick={() => canExpand && setIsExpanded((current) => !current)}
				disabled={!canExpand}
			>
				{canExpand ? (
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
				) : (
					<FileDiff className="size-3 shrink-0" />
				)}
				<span className="min-w-0 flex-1 truncate">
					{preview?.operation ?? "Edit preview unavailable"}
					{filePath ? `: ${filePath}` : ""}
				</span>
				{kind && <ChangeKindBadge kind={kind} />}
				{preview && (
					<span className="shrink-0 tabular-nums">
						<span className="text-green-600 dark:text-green-400">
							+{changedLines.added}
						</span>
						<span className="ml-1 text-red-600 dark:text-red-400">
							-{changedLines.removed}
						</span>
					</span>
				)}
			</button>
			{isExpanded && (
				<>
					{error && (
						<div className="border-t border-border px-2 py-1 text-destructive">
							Preview unavailable: {error}
						</div>
					)}
					{preview?.warnings.map((warning) => (
						<div
							key={warning}
							className="border-t border-border px-2 py-1 text-muted-foreground"
						>
							{warning}
						</div>
					))}
					{preview &&
						preview.hunks.length === 0 &&
						preview.warnings.length === 0 && (
							<div className="border-t border-border px-2 py-1 text-muted-foreground">
								No text changes detected.
							</div>
						)}
					{preview && preview.hunks.length > 0 && (
						<div className="max-h-80 overflow-auto border-t border-border">
							<DiffViewerSection
								isImage={false}
								isMarkdown={false}
								showPreview={false}
								imageDiff={{
									originalUrl: null,
									modifiedUrl: null,
									loading: false,
								}}
								originalContent={preview.originalContent}
								modifiedContent={preview.modifiedContent}
								diffMode="inline"
								diffOnlyMode={true}
								filePath={preview.filePath ?? undefined}
								hunks={diffHunks}
							/>
						</div>
					)}
					{filePath && (
						<div className="flex items-center justify-end gap-1 border-t border-border px-2 py-1">
							{onOpenDiffFile && (
								<button
									type="button"
									className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
									onClick={() =>
										onOpenDiffFile(diffFilePath(worktreePath, filePath))
									}
								>
									<GitCompare className="size-3" />
									<span>Open diff</span>
								</button>
							)}
							<button
								type="button"
								className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
								onClick={handleOpenInEditor}
							>
								{openState === "opened" ? (
									<Check className="size-3" />
								) : (
									<ExternalLink className="size-3" />
								)}
								<span>
									{openState === "error" ? "Open failed" : "Open in editor"}
								</span>
							</button>
						</div>
					)}
				</>
			)}
		</div>
	);
}
