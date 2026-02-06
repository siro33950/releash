import {
	AlignJustify,
	ChevronLeft,
	ChevronRight,
	Minus,
	SplitSquareHorizontal,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { useGitOriginalContent } from "@/hooks/useGitOriginalContent";
import { useHunks } from "@/hooks/useHunks";
import { type Hunk, type ChangeGroup, computeChangeGroups, computeHunks, markStagedGroups } from "@/lib/computeHunks";
import { generateGroupPatch, generatePatch } from "@/lib/generatePatch";
import { cn } from "@/lib/utils";
import type { LineComment } from "@/types/comment";
import type { TabInfo } from "@/types/editor";
import type { Theme } from "@/types/settings";
import { EditorTabs } from "./EditorTabs";
import { EmptyState } from "./EmptyState";
import {
	type DiffBase,
	type DiffMode,
	MonacoDiffViewer,
} from "./MonacoDiffViewer";
import { ReviewPanel } from "./ReviewPanel";

export interface EditorPanelProps {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	onTabClick: (path: string) => void;
	onTabClose: (path: string) => void;
	diffBase: DiffBase;
	diffMode: DiffMode;
	onDiffBaseChange: (base: DiffBase) => void;
	onDiffModeChange: (mode: DiffMode) => void;
	onContentChange?: (path: string, content: string) => void;
	fontSize?: number;
	comments?: LineComment[];
	onAddComment?: (filePath: string, lineNumber: number, content: string, endLine?: number) => void;
	rootPath?: string | null;
	onStageHunk?: (repoPath: string, patch: string) => Promise<void>;
	onUnstageHunk?: (repoPath: string, patch: string) => Promise<void>;
	onSendToTerminal?: (comments: LineComment[]) => void;
	theme?: Theme;
	gitRefreshKey?: number;
	onGitChanged?: () => void;
}

export function EditorPanel({
	tabs,
	activeTab,
	onTabClick,
	onTabClose,
	diffBase,
	diffMode,
	onDiffBaseChange,
	onDiffModeChange,
	onContentChange,
	fontSize,
	comments,
	onAddComment,
	rootPath,
	onStageHunk,
	onSendToTerminal,
	theme,
	gitRefreshKey,
	onGitChanged,
}: EditorPanelProps) {
	const [revealLine, setRevealLine] = useState<{ line: number; key: number } | undefined>();
	const [pendingJump, setPendingJump] = useState<{ filePath: string; lineNumber: number } | null>(null);
	const revealKeyRef = useRef(0);
	const filePath = activeTab?.path ?? null;

	const originalContent = useGitOriginalContent(
		filePath,
		diffBase,
		activeTab?.originalContent ?? "",
		gitRefreshKey,
	);

	const stagedContent = useGitOriginalContent(
		diffBase === "HEAD" ? filePath : null,
		"staged",
		"",
		gitRefreshKey,
	);

	const modifiedContent = activeTab?.content ?? "";

	const { changeGroups: rawChangeGroups, currentIndex, total, goTo } = useHunks(
		originalContent,
		modifiedContent,
		filePath ?? undefined,
	);

	const changeGroups = useMemo(() => {
		if (diffBase !== "HEAD") return rawChangeGroups;
		const hunks = computeHunks(originalContent, modifiedContent, filePath ?? undefined);
		const stagedHunks = computeHunks(originalContent, stagedContent, filePath ?? undefined);
		const stagedGroups = computeChangeGroups(stagedHunks);
		return markStagedGroups(rawChangeGroups, stagedGroups, hunks, stagedHunks);
	}, [rawChangeGroups, diffBase, originalContent, modifiedContent, stagedContent, filePath]);

	const commentRanges = useMemo(() => {
		if (!comments || !filePath) return undefined;
		return comments
			.filter((c) => c.filePath === filePath)
			.map((c) => ({ start: c.lineNumber, end: c.endLine }));
	}, [comments, filePath]);

	const handleAddComment = useCallback(
		(lineNumber: number, content: string, endLine?: number) => {
			if (filePath) {
				onAddComment?.(filePath, lineNumber, content, endLine);
			}
		},
		[filePath, onAddComment],
	);

	const getCommentsForLine = useCallback(
		(lineNumber: number): LineComment[] => {
			if (!comments || !filePath) return [];
			return comments.filter(
				(c) =>
					c.filePath === filePath &&
					(c.lineNumber === lineNumber ||
						(c.endLine != null &&
							lineNumber >= c.lineNumber &&
							lineNumber <= c.endLine)),
			);
		},
		[comments, filePath],
	);

	const getRelativePath = useCallback(() => {
		if (!rootPath || !filePath) return null;
		return filePath.startsWith(`${rootPath}/`)
			? filePath.slice(rootPath.length + 1)
			: filePath;
	}, [rootPath, filePath]);

	const findMatchingGroup = useCallback(
		(targetLines: string[], hunks: Hunk[], groups: ChangeGroup[], reverse = false) => {
			let target: string;
			if (reverse) {
				const newMinus = targetLines.filter(l => l.startsWith("+")).map(l => `-${l.slice(1)}`);
				const newPlus = targetLines.filter(l => l.startsWith("-")).map(l => `+${l.slice(1)}`);
				target = [...newMinus, ...newPlus].join("\n");
			} else {
				target = targetLines.join("\n");
			}
			for (const g of groups) {
				const h = hunks.find((h) => h.index === g.hunkIndex);
				if (!h) continue;
				const lines = h.lines.slice(g.lineOffsetStart, g.lineOffsetEnd + 1).join("\n");
				if (lines === target) return { group: g, hunk: h };
			}
			return null;
		},
		[],
	);

	const handleStageGroup = useCallback(
		async (groupIndex: number) => {
			const relativePath = getRelativePath();
			if (!relativePath || !rootPath) return;
			const allHunks = computeHunks(originalContent, modifiedContent, relativePath);
			const allGroups = computeChangeGroups(allHunks);
			const group = allGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = allHunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			let patchHunk = hunk;
			let patchGroup = group;

			if (diffBase === "HEAD") {
				const targetLines = hunk.lines.slice(group.lineOffsetStart, group.lineOffsetEnd + 1);
				const s2wHunks = computeHunks(stagedContent, modifiedContent, relativePath);
				const s2wGroups = computeChangeGroups(s2wHunks);
				const match = findMatchingGroup(targetLines, s2wHunks, s2wGroups);
				if (!match) return;
				patchHunk = match.hunk;
				patchGroup = match.group;
			}

			const patch = generateGroupPatch(relativePath, patchHunk, patchGroup);
			if (patch) {
				try {
					await onStageHunk?.(rootPath, patch);
					onGitChanged?.();
				} catch (e) {
					console.error("Stage group failed:", e);
				}
			}
		},
		[getRelativePath, rootPath, originalContent, modifiedContent, stagedContent, diffBase, onStageHunk, onGitChanged, findMatchingGroup],
	);

	const handleUnstageGroup = useCallback(
		async (groupIndex: number) => {
			const relativePath = getRelativePath();
			if (!relativePath || !rootPath) return;

			const allHunks = computeHunks(originalContent, modifiedContent, relativePath);
			const allGroups = computeChangeGroups(allHunks);
			const group = allGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = allHunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			const targetLines = hunk.lines.slice(group.lineOffsetStart, group.lineOffsetEnd + 1);
			const s2hHunks = computeHunks(stagedContent, originalContent, relativePath);
			const s2hGroups = computeChangeGroups(s2hHunks);
			const match = findMatchingGroup(targetLines, s2hHunks, s2hGroups, true);
			if (!match) return;

			const patch = generateGroupPatch(relativePath, match.hunk, match.group);
			if (patch) {
				try {
					await onStageHunk?.(rootPath, patch);
					onGitChanged?.();
				} catch (e) {
					console.error("Unstage group failed:", e);
				}
			}
		},
		[getRelativePath, rootPath, originalContent, modifiedContent, stagedContent, onStageHunk, onGitChanged, findMatchingGroup],
	);

	const handleStageAll = useCallback(async () => {
		const relativePath = getRelativePath();
		if (!relativePath || !rootPath) return;
		const base = diffBase === "HEAD" ? stagedContent : originalContent;
		const allHunks = computeHunks(base, modifiedContent, relativePath);
		const allIndices = allHunks.map((h) => h.index);
		const patch = generatePatch(relativePath, allHunks, allIndices);
		if (patch) {
			try {
				await onStageHunk?.(rootPath, patch);
				onGitChanged?.();
			} catch (e) {
				console.error("Stage all failed:", e);
			}
		}
	}, [getRelativePath, rootPath, originalContent, modifiedContent, stagedContent, diffBase, onStageHunk, onGitChanged]);

	const handleUnstageAll = useCallback(async () => {
		const relativePath = getRelativePath();
		if (!relativePath || !rootPath) return;
		const allHunks = computeHunks(stagedContent, originalContent, relativePath);
		const allIndices = allHunks.map((h) => h.index);
		const patch = generatePatch(relativePath, allHunks, allIndices);
		if (patch) {
			try {
				await onStageHunk?.(rootPath, patch);
				onGitChanged?.();
			} catch (e) {
				console.error("Unstage all failed:", e);
			}
		}
	}, [getRelativePath, rootPath, originalContent, stagedContent, onStageHunk, onGitChanged]);

	const revealHunk = useCallback(
		(index: number) => {
			if (index < 0 || index >= changeGroups.length) return;
			goTo(index);
			revealKeyRef.current += 1;
			setRevealLine({ line: changeGroups[index].newStart, key: revealKeyRef.current });
		},
		[changeGroups, goTo],
	);

	const handleGoToNext = useCallback(() => {
		if (changeGroups.length === 0) return;
		const nextIndex = (currentIndex + 1) % changeGroups.length;
		revealHunk(nextIndex);
	}, [changeGroups.length, currentIndex, revealHunk]);

	const handleGoToPrev = useCallback(() => {
		if (changeGroups.length === 0) return;
		const prevIndex = (currentIndex - 1 + changeGroups.length) % changeGroups.length;
		revealHunk(prevIndex);
	}, [changeGroups.length, currentIndex, revealHunk]);

	const handleCommentClick = useCallback(
		(commentFilePath: string, lineNumber: number) => {
			if (commentFilePath === filePath) {
				revealKeyRef.current += 1;
				setRevealLine({ line: lineNumber, key: revealKeyRef.current });
			} else {
				onTabClick(commentFilePath);
				setPendingJump({ filePath: commentFilePath, lineNumber });
			}
		},
		[filePath, onTabClick],
	);

	useEffect(() => {
		if (pendingJump && activeTab?.path === pendingJump.filePath) {
			revealKeyRef.current += 1;
			setRevealLine({ line: pendingJump.lineNumber, key: revealKeyRef.current });
			setPendingJump(null);
		}
	}, [pendingJump, activeTab?.path]);

	const handleContentChange = activeTab
		? (content: string) => onContentChange?.(activeTab.path, content)
		: undefined;

	return (
		<div className="flex flex-col h-full">
			<EditorTabs
				tabs={tabs}
				activeTabPath={activeTab?.path ?? null}
				onTabClick={onTabClick}
				onTabClose={onTabClose}
			/>
			<Group orientation="vertical" className="flex-1">
				<Panel id="editor-main" defaultSize={70} minSize={30}>
					<div className="flex flex-col h-full">
						<div className="flex-1 overflow-hidden">
							{activeTab ? (
								<MonacoDiffViewer
									key={activeTab.path}
									originalContent={originalContent}
									modifiedContent={modifiedContent}
									language={activeTab.language}
									diffMode={diffMode}
									onContentChange={handleContentChange}
									fontSize={fontSize}
									changeGroups={changeGroups}
									commentRanges={commentRanges}
									onStageHunk={handleStageGroup}
									onUnstageHunk={diffBase === "HEAD" ? handleUnstageGroup : undefined}
									onAddComment={handleAddComment}
									getCommentsForLine={getCommentsForLine}
									revealLine={revealLine}
									theme={theme}
								/>
							) : (
								<EmptyState />
							)}
						</div>
						{activeTab && (
							<div className="flex items-center justify-between px-3 py-1.5 border-t border-border bg-card">
								<div className="flex items-center gap-2">
									<span className="text-xs text-muted-foreground">Base:</span>
									<select
										value={diffBase}
										onChange={(e) => onDiffBaseChange(e.target.value as DiffBase)}
										className="bg-muted border border-border rounded px-2 py-0.5 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
									>
										<option value="HEAD">HEAD</option>
										<option value="staged">Staged</option>
																			</select>
									{total > 0 && (
										<div className="flex items-center gap-1 ml-2">
											{onStageHunk && (
												<>
													<button
														type="button"
														onClick={handleStageAll}
														className="px-1.5 py-0.5 text-[10px] bg-status-added/20 text-status-added rounded hover:bg-status-added/30 transition-colors"
													>
														Stage All
													</button>
													{diffBase === "HEAD" && (
														<button
															type="button"
															onClick={handleUnstageAll}
															className="px-1.5 py-0.5 text-[10px] bg-status-modified/20 text-status-modified rounded hover:bg-status-modified/30 transition-colors"
														>
															Unstage All
														</button>
													)}
												</>
											)}
											<button
												type="button"
												onClick={handleGoToPrev}
												className="p-0.5 rounded hover:bg-muted transition-colors text-muted-foreground hover:text-foreground"
												title="Previous hunk"
											>
												<ChevronLeft className="h-3.5 w-3.5" />
											</button>
											<button
												type="button"
												onClick={handleGoToNext}
												className="p-0.5 rounded hover:bg-muted transition-colors text-muted-foreground hover:text-foreground"
												title="Next hunk"
											>
												<ChevronRight className="h-3.5 w-3.5" />
											</button>
											<span className="text-[10px] text-muted-foreground font-mono">
												{currentIndex + 1}/{total}
											</span>
										</div>
									)}
								</div>
								<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
									<button
										type="button"
										onClick={() => onDiffModeChange("gutter")}
										className={cn(
											"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
											diffMode === "gutter"
												? "bg-background shadow-sm text-foreground"
												: "text-muted-foreground hover:text-foreground",
										)}
										title="Gutter markers only"
									>
										<Minus className="h-3.5 w-3.5" />
										Gutter
									</button>
									<button
										type="button"
										onClick={() => onDiffModeChange("inline")}
										className={cn(
											"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
											diffMode === "inline"
												? "bg-background shadow-sm text-foreground"
												: "text-muted-foreground hover:text-foreground",
										)}
										title="Inline diff"
									>
										<AlignJustify className="h-3.5 w-3.5" />
										Inline
									</button>
									<button
										type="button"
										onClick={() => onDiffModeChange("split")}
										className={cn(
											"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
											diffMode === "split"
												? "bg-background shadow-sm text-foreground"
												: "text-muted-foreground hover:text-foreground",
										)}
										title="Split view"
									>
										<SplitSquareHorizontal className="h-3.5 w-3.5" />
										Split
									</button>
								</div>
							</div>
						)}
					</div>
				</Panel>
				<Separator className="h-px bg-border hover:bg-primary/50 cursor-row-resize" />
				<Panel id="review" defaultSize={30} minSize={15} collapsible>
					<ReviewPanel
						comments={comments ?? []}
						onCommentClick={handleCommentClick}
						onSendToTerminal={onSendToTerminal}
					/>
				</Panel>
			</Group>
		</div>
	);
}
