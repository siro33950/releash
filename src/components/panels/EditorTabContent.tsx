import {
	AlignJustify,
	ChevronLeft,
	ChevronRight,
	Eye,
	FileCode,
	Minus,
	SplitSquareHorizontal,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useEditorContext } from "@/contexts/EditorContext";
import { useGitOriginalContent } from "@/hooks/useGitOriginalContent";
import { useHunks } from "@/hooks/useHunks";
import { useImageDiff } from "@/hooks/useImageDiff";
import {
	type ChangeGroup,
	computeChangeGroups,
	computeHunks,
	type Hunk,
	markStagedGroups,
} from "@/lib/computeHunks";
import { generateGroupPatch, generatePatch } from "@/lib/generatePatch";
import { isImageFile } from "@/lib/imageUtils";
import { isMarkdownFile } from "@/lib/markdownUtils";
import { cn } from "@/lib/utils";
import type { LineComment } from "@/types/comment";
import type { DiffBase } from "@/types/settings";
import { Breadcrumb } from "./Breadcrumb";
import { EmptyState } from "./EmptyState";
import { ImageDiffViewer } from "./ImageDiffViewer";
import { MarkdownDiffViewer } from "./MarkdownDiffViewer";
import { MonacoDiffViewer } from "./MonacoDiffViewer";

export interface EditorTabContentProps {
	filePath: string;
	externalRevealLine?: { path: string; line: number } | null;
	onExternalRevealConsumed?: () => void;
}

export function EditorTabContent({
	filePath,
	externalRevealLine,
	onExternalRevealConsumed,
}: EditorTabContentProps) {
	const {
		getFileContent,
		updateContent,
		diffBase,
		diffMode,
		setDiffBase,
		setDiffMode,
		comments,
		addComment,
		showSentComments,
		rootPath,
		onStageHunk,
		onGitChanged,
		gitRefreshKey,
		theme,
		fontSize,
		onSearchOccurrences,
	} = useEditorContext();
	const fileContent = getFileContent(filePath);
	const isImage = isImageFile(filePath);
	const isMarkdown = isMarkdownFile(filePath);
	const [showPreview, setShowPreview] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: reset preview when file changes
	useEffect(() => {
		setShowPreview(false);
	}, [filePath]);

	const [revealLine, setRevealLine] = useState<
		{ line: number; key: number } | undefined
	>();
	const revealKeyRef = useRef(0);

	const originalContent = useGitOriginalContent(
		isImage ? null : filePath,
		diffBase,
		fileContent?.originalContent ?? "",
		gitRefreshKey,
	);

	const stagedContent = useGitOriginalContent(
		!isImage && diffBase === "HEAD" ? filePath : null,
		"staged",
		"",
		gitRefreshKey,
	);

	const imageDiff = useImageDiff(
		isImage ? filePath : null,
		diffBase,
		gitRefreshKey,
	);

	const modifiedContent = fileContent?.content ?? "";

	const {
		changeGroups: rawChangeGroups,
		currentIndex,
		total,
		goTo,
	} = useHunks(originalContent, modifiedContent, filePath);

	const changeGroups = useMemo(() => {
		if (diffBase !== "HEAD") return rawChangeGroups;
		const hunks = computeHunks(originalContent, modifiedContent, filePath);
		const stagedHunks = computeHunks(originalContent, stagedContent, filePath);
		const stagedGroups = computeChangeGroups(stagedHunks);
		return markStagedGroups(rawChangeGroups, stagedGroups, hunks, stagedHunks);
	}, [
		rawChangeGroups,
		diffBase,
		originalContent,
		modifiedContent,
		stagedContent,
		filePath,
	]);

	const commentRanges = useMemo(() => {
		return comments
			.filter(
				(c) =>
					c.filePath === filePath && (showSentComments || c.status !== "sent"),
			)
			.map((c) => ({ start: c.lineNumber, end: c.endLine }));
	}, [comments, filePath, showSentComments]);

	const handleAddComment = useCallback(
		(lineNumber: number, content: string, endLine?: number) => {
			addComment(filePath, lineNumber, content, endLine);
		},
		[filePath, addComment],
	);

	const getCommentsForLine = useCallback(
		(lineNumber: number): LineComment[] => {
			return comments.filter(
				(c) =>
					c.filePath === filePath &&
					(showSentComments || c.status !== "sent") &&
					(c.lineNumber === lineNumber ||
						(c.endLine != null &&
							lineNumber >= c.lineNumber &&
							lineNumber <= c.endLine)),
			);
		},
		[comments, filePath, showSentComments],
	);

	const getRelativePath = useCallback(() => {
		if (!rootPath) return null;
		return filePath.startsWith(`${rootPath}/`)
			? filePath.slice(rootPath.length + 1)
			: filePath;
	}, [rootPath, filePath]);

	const findMatchingGroup = useCallback(
		(
			targetLines: string[],
			hunks: Hunk[],
			groups: ChangeGroup[],
			reverse = false,
		) => {
			let target: string;
			if (reverse) {
				const newMinus = targetLines
					.filter((l) => l.startsWith("+"))
					.map((l) => `-${l.slice(1)}`);
				const newPlus = targetLines
					.filter((l) => l.startsWith("-"))
					.map((l) => `+${l.slice(1)}`);
				target = [...newMinus, ...newPlus].join("\n");
			} else {
				target = targetLines.join("\n");
			}
			for (const g of groups) {
				const h = hunks.find((h) => h.index === g.hunkIndex);
				if (!h) continue;
				const lines = h.lines
					.slice(g.lineOffsetStart, g.lineOffsetEnd + 1)
					.join("\n");
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
			const allHunks = computeHunks(
				originalContent,
				modifiedContent,
				relativePath,
			);
			const allGroups = computeChangeGroups(allHunks);
			const group = allGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = allHunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			let patchHunk = hunk;
			let patchGroup = group;

			if (diffBase === "HEAD") {
				const targetLines = hunk.lines.slice(
					group.lineOffsetStart,
					group.lineOffsetEnd + 1,
				);
				const s2wHunks = computeHunks(
					stagedContent,
					modifiedContent,
					relativePath,
				);
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
		[
			getRelativePath,
			rootPath,
			originalContent,
			modifiedContent,
			stagedContent,
			diffBase,
			onStageHunk,
			onGitChanged,
			findMatchingGroup,
		],
	);

	const handleUnstageGroup = useCallback(
		async (groupIndex: number) => {
			const relativePath = getRelativePath();
			if (!relativePath || !rootPath) return;
			const allHunks = computeHunks(
				originalContent,
				modifiedContent,
				relativePath,
			);
			const allGroups = computeChangeGroups(allHunks);
			const group = allGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = allHunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			const targetLines = hunk.lines.slice(
				group.lineOffsetStart,
				group.lineOffsetEnd + 1,
			);
			const s2hHunks = computeHunks(
				stagedContent,
				originalContent,
				relativePath,
			);
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
		[
			getRelativePath,
			rootPath,
			originalContent,
			modifiedContent,
			stagedContent,
			onStageHunk,
			onGitChanged,
			findMatchingGroup,
		],
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
	}, [
		getRelativePath,
		rootPath,
		originalContent,
		modifiedContent,
		stagedContent,
		diffBase,
		onStageHunk,
		onGitChanged,
	]);

	const handleUnstageAll = useCallback(async () => {
		const relativePath = getRelativePath();
		if (!relativePath || !rootPath || !stagedContent) return;
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
	}, [
		getRelativePath,
		rootPath,
		originalContent,
		stagedContent,
		onStageHunk,
		onGitChanged,
	]);

	const revealHunk = useCallback(
		(index: number) => {
			if (index < 0 || index >= changeGroups.length) return;
			goTo(index);
			revealKeyRef.current += 1;
			setRevealLine({
				line: changeGroups[index].newStart,
				key: revealKeyRef.current,
			});
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
		const prevIndex =
			(currentIndex - 1 + changeGroups.length) % changeGroups.length;
		revealHunk(prevIndex);
	}, [changeGroups.length, currentIndex, revealHunk]);

	useEffect(() => {
		if (externalRevealLine && externalRevealLine.path === filePath) {
			revealKeyRef.current += 1;
			setRevealLine({
				line: externalRevealLine.line,
				key: revealKeyRef.current,
			});
			onExternalRevealConsumed?.();
		}
	}, [externalRevealLine, filePath, onExternalRevealConsumed]);

	const handleContentChange = useCallback(
		(content: string) => updateContent(filePath, content),
		[filePath, updateContent],
	);

	if (!fileContent) {
		return <EmptyState />;
	}

	return (
		<div className="absolute inset-0 flex flex-col">
			<Breadcrumb rootPath={rootPath} filePath={filePath}>
				{isMarkdown && (
					<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
						<button
							type="button"
							onClick={() => setShowPreview(false)}
							className={cn(
								"flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] transition-colors",
								!showPreview
									? "bg-background shadow-sm text-foreground"
									: "text-muted-foreground hover:text-foreground",
							)}
							title="Editor"
						>
							<FileCode className="h-3 w-3" />
							Editor
						</button>
						<button
							type="button"
							onClick={() => setShowPreview(true)}
							className={cn(
								"flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] transition-colors",
								showPreview
									? "bg-background shadow-sm text-foreground"
									: "text-muted-foreground hover:text-foreground",
							)}
							title="Preview"
						>
							<Eye className="h-3 w-3" />
							Preview
						</button>
					</div>
				)}
			</Breadcrumb>
			<div
				className="flex-1 min-h-0"
				style={{ position: "relative", overflow: "hidden" }}
			>
				{isImage ? (
					<ImageDiffViewer
						originalUrl={imageDiff.originalUrl}
						modifiedUrl={imageDiff.modifiedUrl}
						loading={imageDiff.loading}
					/>
				) : isMarkdown && showPreview ? (
					<MarkdownDiffViewer
						originalContent={originalContent}
						modifiedContent={modifiedContent}
					/>
				) : (
					<MonacoDiffViewer
						key={filePath}
						originalContent={originalContent}
						modifiedContent={modifiedContent}
						language={fileContent.language}
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
						filePath={filePath}
						onSearchOccurrences={onSearchOccurrences}
					/>
				)}
			</div>
			{!isImage && (
				<div className="flex items-center justify-between px-3 py-1.5 border-t border-border bg-card">
					<div className="flex items-center gap-2">
						<span className="text-xs text-muted-foreground">Base:</span>
						<select
							value={diffBase}
							onChange={(e) => setDiffBase(e.target.value as DiffBase)}
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
							onClick={() => setDiffMode("gutter")}
							className={cn(
								"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
								diffMode === "gutter" || (showPreview && isMarkdown)
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
							onClick={() => setDiffMode("inline")}
							disabled={showPreview && isMarkdown}
							className={cn(
								"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
								showPreview && isMarkdown
									? "text-muted-foreground/50 cursor-not-allowed"
									: diffMode === "inline"
										? "bg-background shadow-sm text-foreground"
										: "text-muted-foreground hover:text-foreground",
							)}
							title={
								showPreview && isMarkdown
									? "Preview mode supports Gutter only"
									: "Inline diff"
							}
						>
							<AlignJustify className="h-3.5 w-3.5" />
							Inline
						</button>
						<button
							type="button"
							onClick={() => setDiffMode("split")}
							disabled={showPreview && isMarkdown}
							className={cn(
								"flex items-center gap-1 px-2 py-0.5 rounded text-xs transition-colors",
								showPreview && isMarkdown
									? "text-muted-foreground/50 cursor-not-allowed"
									: diffMode === "split"
										? "bg-background shadow-sm text-foreground"
										: "text-muted-foreground hover:text-foreground",
							)}
							title={
								showPreview && isMarkdown
									? "Preview mode supports Gutter only"
									: "Split view"
							}
						>
							<SplitSquareHorizontal className="h-3.5 w-3.5" />
							Split
						</button>
					</div>
				</div>
			)}
		</div>
	);
}
