import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useEditorContext } from "@/contexts/EditorContext";
import { useGitOriginalContent } from "@/hooks/useGitOriginalContent";
import { useHunks } from "@/hooks/useHunks";
import { useImageDiff } from "@/hooks/useImageDiff";
import {
	computeChangeGroups,
	computeHunks,
	markStagedGroups,
} from "@/lib/computeHunks";
import { isImageFile } from "@/lib/imageUtils";
import { isMarkdownFile } from "@/lib/markdownUtils";
import type { LineComment } from "@/types/comment";
import { Breadcrumb } from "./Breadcrumb";
import { DiffToolbar } from "./DiffToolbar";
import { DiffViewerSection } from "./DiffViewerSection";
import { EmptyState } from "./EmptyState";
import { PreviewToggle } from "./PreviewToggle";
import { useDiffOperations } from "./useDiffOperations";

export interface EditorTabContentProps {
	filePath: string;
	externalRevealLine?: {
		path: string;
		line: number;
		openThread?: boolean;
	} | null;
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
		deleteComment,
		resolveComment,
		updateComment: updateCommentContent,
		copyComment,
		showResolvedComments,
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

	const [revealLine, setRevealLine] = useState<
		{ line: number; key: number; openThread?: boolean } | undefined
	>();
	const revealKeyRef = useRef(0);

	const originalContent = useGitOriginalContent(
		isImage ? null : filePath,
		diffBase,
		fileContent?.originalContent ?? "",
		gitRefreshKey,
	);

	const stagedContent = useGitOriginalContent(
		!isImage && diffBase === "branch-base" ? filePath : null,
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
		if (diffBase !== "branch-base") return rawChangeGroups;
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

	const relativeFilePath = useMemo(() => {
		if (rootPath && filePath.startsWith(`${rootPath}/`)) {
			return filePath.slice(rootPath.length + 1);
		}
		return filePath;
	}, [rootPath, filePath]);

	const commentRanges = useMemo(() => {
		return comments
			.filter(
				(c) =>
					c.filePath === relativeFilePath &&
					(showResolvedComments || !c.resolved),
			)
			.map((c) => ({ start: c.lineNumber, end: c.endLine }));
	}, [comments, relativeFilePath, showResolvedComments]);

	const handleAddComment = useCallback(
		(lineNumber: number, content: string, endLine?: number) => {
			addComment(relativeFilePath, lineNumber, content, endLine);
		},
		[relativeFilePath, addComment],
	);

	const getCommentsForLine = useCallback(
		(lineNumber: number): LineComment[] => {
			return comments.filter(
				(c) =>
					c.filePath === relativeFilePath &&
					(showResolvedComments || !c.resolved) &&
					(c.lineNumber === lineNumber ||
						(c.endLine != null &&
							lineNumber >= c.lineNumber &&
							lineNumber <= c.endLine)),
			);
		},
		[comments, relativeFilePath, showResolvedComments],
	);

	const {
		handleStageGroup,
		handleUnstageGroup,
		handleStageAll,
		handleUnstageAll,
	} = useDiffOperations({
		filePath,
		rootPath,
		originalContent,
		modifiedContent,
		stagedContent,
		diffBase,
		onStageHunk,
		onGitChanged,
	});

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
				openThread: externalRevealLine.openThread,
			});
			onExternalRevealConsumed?.();
		}
	}, [externalRevealLine, filePath, onExternalRevealConsumed]);

	const handleContentChange = useCallback(
		(content: string) => updateContent(filePath, content),
		[filePath, updateContent],
	);

	if (!fileContent) {
		return (
			<EmptyState
				title="No file selected"
				description="Select a file from the explorer to view its contents"
			/>
		);
	}

	return (
		<div className="absolute inset-0 flex flex-col">
			<Breadcrumb rootPath={rootPath} filePath={filePath}>
				{isMarkdown && (
					<PreviewToggle
						showPreview={showPreview}
						onShowPreviewChange={setShowPreview}
					/>
				)}
			</Breadcrumb>
			<div className="flex-1 min-h-0 relative overflow-hidden">
				<DiffViewerSection
					isImage={isImage}
					isMarkdown={isMarkdown}
					showPreview={showPreview}
					imageDiff={imageDiff}
					originalContent={originalContent}
					modifiedContent={modifiedContent}
					diffMode={diffMode}
					filePath={filePath}
					language={fileContent.language}
					fontSize={fontSize}
					changeGroups={changeGroups}
					commentRanges={commentRanges}
					onContentChange={handleContentChange}
					onStageHunk={handleStageGroup}
					onUnstageHunk={
						diffBase === "branch-base" ? handleUnstageGroup : undefined
					}
					onAddComment={handleAddComment}
					onDeleteComment={deleteComment}
					onResolveComment={resolveComment}
					onUpdateComment={updateCommentContent}
					onCopyComment={copyComment}
					getCommentsForLine={getCommentsForLine}
					revealLine={revealLine}
					theme={theme}
					onSearchOccurrences={onSearchOccurrences}
				/>
			</div>
			{!isImage && (
				<DiffToolbar
					diffBase={diffBase}
					diffMode={diffMode}
					currentIndex={currentIndex}
					total={total}
					onDiffBaseChange={setDiffBase}
					onDiffModeChange={setDiffMode}
					onGoToPrev={handleGoToPrev}
					onGoToNext={handleGoToNext}
					onStageAll={handleStageAll}
					onUnstageAll={handleUnstageAll}
					showStageButtons={!!onStageHunk}
				/>
			)}
		</div>
	);
}
