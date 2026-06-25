import { invoke } from "@tauri-apps/api/core";
import {
	Code,
	Eye,
	GitBranch,
	GitCommitHorizontal,
	PanelLeftClose,
	PanelLeftOpen,
	SquareArrowOutUpRight,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	Separator,
} from "react-resizable-panels";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { useDiffComments } from "@/hooks/useDiffComments";
import { useFileNavigation } from "@/hooks/useFileNavigation";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitEventRefresh } from "@/hooks/useGitEventRefresh";
import { useReviewFileView } from "@/hooks/useReviewFileView";
import { useReviewPanel } from "@/hooks/useReviewPanel";
import { useReviewSnapshot } from "@/hooks/useReviewSnapshot";
import { isMarkdownFile } from "@/lib/markdownUtils";
import { cn } from "@/lib/utils";
import type { ThreadNavigationTarget } from "@/types/diffComment";
import type { MentionReference } from "@/types/session";
import type { DiffBase, DiffMode, DiffSection } from "@/types/settings";
import { Breadcrumb } from "./Breadcrumb";
import { FileCommentPopoverTrigger } from "./DiffFileComment";
import { DiffFileTree } from "./DiffFileTree";
import { DiffToolbar } from "./DiffToolbar";
import { DiffViewerSection } from "./DiffViewerSection";
import { useDiffOperations } from "./useDiffOperations";

interface ReviewPanelProps {
	rootPath: string;
	defaultDiffBase?: DiffBase;
	defaultDiffMode?: DiffMode;
	diffOnlyMode: boolean;
	onDiffOnlyModeChange: (enabled: boolean) => void;
	navigateToThread?: ThreadNavigationTarget | null;
	onSendToAgent?: (
		message: string,
		mentions?: MentionReference[],
	) => Promise<void>;
	initialSelectedFile?: string | null;
	onSelectedFileChange?: (file: string | null) => void;
	onLineRangeSelected?: (
		filePath: string,
		startLine: number,
		endLine: number,
	) => void;
}

function DiffBaseToggle({
	diffBase,
	onDiffBaseChange,
}: {
	diffBase: DiffBase;
	onDiffBaseChange: (base: DiffBase) => void;
}) {
	return (
		<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon-xs"
						aria-label="HEAD"
						aria-pressed={diffBase === "head"}
						onClick={() => onDiffBaseChange("head")}
						className={cn(
							"w-6 h-5",
							diffBase === "head"
								? "bg-background shadow-sm text-foreground"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						<GitCommitHorizontal className="h-3.5 w-3.5" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					HEAD
				</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon-xs"
						aria-label="Branch Base"
						aria-pressed={diffBase === "branch-base"}
						onClick={() => onDiffBaseChange("branch-base")}
						className={cn(
							"w-6 h-5",
							diffBase === "branch-base"
								? "bg-background shadow-sm text-foreground"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						<GitBranch className="h-3.5 w-3.5" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					Branch Base
				</TooltipContent>
			</Tooltip>
		</div>
	);
}

export function ReviewPanel({
	rootPath,
	defaultDiffBase,
	defaultDiffMode,
	diffOnlyMode,
	onDiffOnlyModeChange,
	navigateToThread,
	initialSelectedFile,
	onSelectedFileChange,
	onLineRangeSelected,
}: ReviewPanelProps) {
	const {
		diffBase,
		diffMode,
		selectedFile,
		selectedSection,
		setDiffBase,
		setDiffMode,
		selectFile: selectFileInternal,
	} = useReviewPanel({
		initialDiffBase: defaultDiffBase,
		initialDiffMode: defaultDiffMode,
		initialSelectedFile,
	});
	const previousSelectedFileRef = useRef<string | null>(selectedFile);

	useEffect(() => {
		const previous = previousSelectedFileRef.current;
		if (previous && previous !== selectedFile) {
			onLineRangeSelected?.("", 0, 0);
		}
		previousSelectedFileRef.current = selectedFile;
	}, [selectedFile, onLineRangeSelected]);

	const selectFile = useCallback(
		(path: string | null, section?: DiffSection) => {
			selectFileInternal(path, section);
			onSelectedFileChange?.(path);
			if (path !== selectedFile) {
				onLineRangeSelected?.("", 0, 0);
			}
		},
		[
			selectFileInternal,
			onSelectedFileChange,
			onLineRangeSelected,
			selectedFile,
		],
	);

	const [gitRefreshKey, setGitRefreshKey] = useState(0);

	const handleGitEventRefresh = useCallback(() => {
		setGitRefreshKey((k) => k + 1);
	}, []);
	useGitEventRefresh(rootPath, handleGitEventRefresh);

	// Review file list read model
	const {
		stagedFiles,
		changedFiles,
		stagedTree,
		changesTree,
		stagedFileCount,
		changesFileCount,
		branchBaseTree,
		branchBaseFileCount,
		version: reviewSnapshotVersion,
		refresh: refreshReviewSnapshot,
	} = useReviewSnapshot(rootPath, diffBase, gitRefreshKey);

	const totalFileCount =
		diffBase === "branch-base"
			? branchBaseFileCount
			: stagedFileCount + changesFileCount;

	// Breadcrumb segments
	const breadcrumbSegments = useMemo(() => {
		if (!selectedFile) return [];
		const parts = selectedFile.split("/");
		return parts.map((name, i) => ({
			isFile: i === parts.length - 1,
			name,
		}));
	}, [selectedFile]);

	// File navigation — combined list of all changed files (not scoped to section)
	const navigationTree = useMemo(() => {
		if (diffBase === "branch-base") {
			return branchBaseTree;
		}
		return [...stagedTree, ...changesTree];
	}, [diffBase, branchBaseTree, stagedTree, changesTree]);

	const { fileNavigation, goToPrevFile, goToNextFile } = useFileNavigation(
		navigationTree,
		selectedFile,
	);

	const determineSectionForFile = useCallback(
		(path: string): DiffSection => {
			if (diffBase === "branch-base") return "changes";
			const inStaged = stagedFiles.some((f) => f.path === path);
			const inChanges = changedFiles.some((f) => f.path === path);
			if (inStaged && !inChanges) return "staged";
			if (!inStaged && inChanges) return "changes";
			return selectedSection;
		},
		[diffBase, stagedFiles, changedFiles, selectedSection],
	);

	const [scrollToLine, setScrollToLine] = useState<number | null>(null);
	const [scrollToThread, setScrollToThread] = useState<string | null>(null);
	const [openFileCommentsForFile, setOpenFileCommentsForFile] = useState<
		string | null
	>(null);
	// General threads (file 非依存) のポップオーバー制御。`navigateToThread` が
	// general thread を指す場合に、両ヘッダー位置の "General threads" ポップオーバーを
	// プログラム的に開けるようにする。
	const [openGeneralComments, setOpenGeneralComments] = useState(false);

	useEffect(() => {
		if (!navigateToThread) return;
		const { filePath, threadId, lineNumber, isFileComment } = navigateToThread;
		const isGeneral = !filePath;
		if (!isGeneral) {
			const section = determineSectionForFile(filePath);
			selectFile(filePath, section);
		}
		if (isFileComment) {
			setScrollToLine(null);
			setScrollToThread(null);
			setOpenGeneralComments(isGeneral);
			setOpenFileCommentsForFile(isGeneral ? null : filePath);
		} else {
			setOpenGeneralComments(false);
			setOpenFileCommentsForFile(null);
			setScrollToLine(lineNumber ?? null);
			setScrollToThread(threadId);
		}
	}, [navigateToThread, determineSectionForFile, selectFile]);

	const handleGoToPrevFile = useCallback(() => {
		const prev = goToPrevFile();
		if (prev) {
			selectFile(prev, determineSectionForFile(prev));
		}
	}, [goToPrevFile, selectFile, determineSectionForFile]);

	const handleGoToNextFile = useCallback(() => {
		const next = goToNextFile();
		if (next) {
			selectFile(next, determineSectionForFile(next));
		}
	}, [goToNextFile, selectFile, determineSectionForFile]);

	// File diff content
	const selectedFilePath = selectedFile ? `${rootPath}/${selectedFile}` : null;
	const {
		view: reviewFileView,
		originalContent,
		modifiedContent,
		hunks,
		changeGroups,
		imageDiff,
		error: reviewFileViewError,
	} = useReviewFileView(
		rootPath,
		selectedFile,
		diffBase,
		selectedSection,
		gitRefreshKey,
		reviewSnapshotVersion,
	);
	const fallbackView =
		reviewFileView?.kind === "fallback" ? reviewFileView : null;
	const binaryView = reviewFileView?.kind === "binary" ? reviewFileView : null;
	const isTextDiff = reviewFileView?.kind === "textDiff";

	// Image / Markdown detection
	const isImage = reviewFileView?.kind === "image";
	const isMarkdown =
		isTextDiff && selectedFile ? isMarkdownFile(selectedFile) : false;
	const [showMarkdownPreview, setShowMarkdownPreview] = useState(false);

	// Diff comments
	const worktreeName = rootPath;

	const {
		comments: allComments,
		addComment,
		appendComment,
		resolveThread,
		deleteThread,
		getCommentsForFile,
	} = useDiffComments({ worktreeName });

	const fileComments = useMemo(
		() => (selectedFile ? getCommentsForFile(selectedFile) : []),
		[selectedFile, getCommentsForFile],
	);

	const lineComments = useMemo(
		() => fileComments.filter((c) => c.target.lineNumber != null),
		[fileComments],
	);

	const handleAddLineComment = useCallback(
		async (lineNumber: number, content: string) => {
			if (!selectedFile) return;
			await addComment({
				filePath: selectedFile,
				lineNumber,
				content,
			});
		},
		[selectedFile, addComment],
	);

	const handleAddRangeComment = useCallback(
		async (startLine: number, endLine: number, content: string) => {
			if (!selectedFile) return;
			await addComment({
				filePath: selectedFile,
				lineNumber: startLine,
				endLine,
				content,
			});
		},
		[selectedFile, addComment],
	);

	const handleAddFileComment = useCallback(
		async (content: string) => {
			if (!selectedFile) return;
			await addComment({
				filePath: selectedFile,
				content,
			});
		},
		[selectedFile, addComment],
	);

	const handleAddGeneralComment = useCallback(
		async (content: string) => {
			await addComment({ content });
		},
		[addComment],
	);

	// File tree panel collapse
	const diffFilesPanelRef = useRef<PanelImperativeHandle>(null);
	const [fileTreeCollapsed, setFileTreeCollapsed] = useState(false);

	const handleToggleFileTree = useCallback(() => {
		const panel = diffFilesPanelRef.current;
		if (!panel) return;
		if (panel.isCollapsed()) {
			panel.expand();
		} else {
			panel.collapse();
		}
	}, []);

	const handleDiffBaseChange = useCallback(
		(base: DiffBase) => {
			if (base === diffBase) return;
			setDiffBase(base);
			selectFile(null, "changes");
		},
		[diffBase, setDiffBase, selectFile],
	);

	// Stage/Unstage actions
	const { stage, unstage } = useGitActions();

	const refreshAfterAction = useCallback(() => {
		setGitRefreshKey((k) => k + 1);
		refreshReviewSnapshot();
	}, [refreshReviewSnapshot]);

	// Determine action label and handler based on section
	const isBranchBase = diffBase === "branch-base";
	const groupActionLabel = selectedSection === "staged" ? "Unstage" : "Stage";

	const diffOps = useDiffOperations({
		rootPath,
		filePath: selectedFile,
		section: selectedSection,
		base: diffBase,
		onGitChanged: refreshAfterAction,
	});

	const handleGroupAction =
		selectedSection === "staged"
			? diffOps.handleUnstageGroup
			: diffOps.handleStageGroup;
	const canUseGroupActions =
		!isBranchBase &&
		reviewFileView?.kind === "textDiff" &&
		!reviewFileView.stale;

	const handleStageFile = useCallback(
		async (path: string) => {
			if (!rootPath) return;
			await stage(rootPath, [path]);
			if (selectedFile === path) {
				selectFile(path, "staged");
			}
			refreshAfterAction();
		},
		[rootPath, selectedFile, selectFile, stage, refreshAfterAction],
	);

	const handleUnstageFile = useCallback(
		async (path: string) => {
			if (!rootPath) return;
			await unstage(rootPath, [path]);
			if (selectedFile === path) {
				selectFile(path, "changes");
			}
			refreshAfterAction();
		},
		[rootPath, selectedFile, selectFile, unstage, refreshAfterAction],
	);

	const handleStageAll = useCallback(async () => {
		if (!rootPath) return;
		const paths = changedFiles.map((f) => f.path);
		if (paths.length === 0) return;
		await stage(rootPath, paths);
		if (selectedFile && paths.includes(selectedFile)) {
			selectFile(selectedFile, "staged");
		}
		refreshAfterAction();
	}, [
		rootPath,
		changedFiles,
		selectedFile,
		selectFile,
		stage,
		refreshAfterAction,
	]);

	const handleUnstageAll = useCallback(async () => {
		if (!rootPath) return;
		const paths = stagedFiles.map((f) => f.path);
		if (paths.length === 0) return;
		await unstage(rootPath, paths);
		if (selectedFile && paths.includes(selectedFile)) {
			selectFile(selectedFile, "changes");
		}
		refreshAfterAction();
	}, [
		rootPath,
		stagedFiles,
		selectedFile,
		selectFile,
		unstage,
		refreshAfterAction,
	]);

	// Handle file selection with section
	const handleSelectFile = useCallback(
		(path: string, section: DiffSection) => {
			selectFile(path, section);
		},
		[selectFile],
	);

	const handleLineRangeSelected = useCallback(
		(startLine: number, endLine: number) => {
			if (!selectedFile) return;
			onLineRangeSelected?.(selectedFile, startLine, endLine);
		},
		[selectedFile, onLineRangeSelected],
	);

	// Empty state
	if (totalFileCount === 0) {
		return (
			<div className="flex flex-col h-full">
				<div className="flex items-center justify-between px-2 h-[32px] border-b border-border bg-card shrink-0">
					<div className="w-5" />
					<div className="flex items-center gap-1">
						<FileCommentPopoverTrigger
							comments={allComments}
							title="General threads"
							addLabel="Add general thread"
							onAdd={handleAddGeneralComment}
							onAppend={appendComment}
							onResolve={resolveThread}
							onDelete={deleteThread}
							open={openGeneralComments}
							onOpenChange={setOpenGeneralComments}
						/>
						<DiffBaseToggle
							diffBase={diffBase}
							onDiffBaseChange={handleDiffBaseChange}
						/>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon-xs"
									onClick={() => {
										invoke("open_folder_in_editor", {
											folderPath: rootPath,
										}).catch((e: unknown) => {
											console.error("Failed to open folder in editor:", e);
										});
									}}
									className="h-5 w-5 text-muted-foreground hover:text-foreground"
									aria-label="Open in editor"
								>
									<SquareArrowOutUpRight className="h-3.5 w-3.5" />
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom" className="text-xs">
								Open in editor
							</TooltipContent>
						</Tooltip>
					</div>
				</div>
				<div className="flex-1 flex items-center justify-center text-muted-foreground text-xs">
					No changes
				</div>
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full">
			{/* Header */}
			<div className="flex items-center justify-between px-2 h-[32px] border-b border-border bg-card shrink-0">
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon-xs"
							onClick={handleToggleFileTree}
							aria-label={
								fileTreeCollapsed ? "Show file list" : "Hide file list"
							}
							className="h-5 w-5 text-muted-foreground hover:text-foreground"
						>
							{fileTreeCollapsed ? (
								<PanelLeftOpen className="h-3.5 w-3.5" />
							) : (
								<PanelLeftClose className="h-3.5 w-3.5" />
							)}
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="text-xs">
						{fileTreeCollapsed ? "Show file list" : "Hide file list"}
					</TooltipContent>
				</Tooltip>
				<div className="flex items-center gap-1">
					<FileCommentPopoverTrigger
						comments={allComments}
						title="General threads"
						addLabel="Add general thread"
						onAdd={handleAddGeneralComment}
						onAppend={appendComment}
						onResolve={resolveThread}
						onDelete={deleteThread}
						open={openGeneralComments}
						onOpenChange={setOpenGeneralComments}
					/>
					<DiffBaseToggle
						diffBase={diffBase}
						onDiffBaseChange={handleDiffBaseChange}
					/>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon-xs"
								onClick={() => {
									invoke("open_folder_in_editor", {
										folderPath: rootPath,
									}).catch((e: unknown) => {
										console.error("Failed to open folder in editor:", e);
									});
								}}
								className="h-5 w-5 text-muted-foreground hover:text-foreground"
								aria-label="Open in editor"
							>
								<SquareArrowOutUpRight className="h-3.5 w-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							Open in editor
						</TooltipContent>
					</Tooltip>
				</div>
			</div>
			{/* Content: split left (file tree) / right (diff viewer) */}
			<div className="flex-1 overflow-hidden">
				<Group orientation="horizontal">
					<Panel
						id="diff-files"
						panelRef={diffFilesPanelRef}
						defaultSize={250}
						minSize={250}
						groupResizeBehavior="preserve-pixel-size"
						collapsible
						onResize={(size) => setFileTreeCollapsed(size.asPercentage <= 0)}
					>
						<div className="h-full overflow-hidden border-r border-border">
							<DiffFileTree
								rootPath={rootPath}
								stagedTree={stagedTree}
								changesTree={changesTree}
								branchBaseTree={branchBaseTree}
								stagedFileCount={stagedFileCount}
								changesFileCount={changesFileCount}
								diffBase={diffBase}
								selectedFile={selectedFile}
								selectedSection={selectedSection}
								onSelectFile={handleSelectFile}
								onStageFile={handleStageFile}
								onUnstageFile={handleUnstageFile}
								onStageAll={handleStageAll}
								onUnstageAll={handleUnstageAll}
							/>
						</div>
					</Panel>
					<Separator />
					<Panel id="diff-view" defaultSize="70%" minSize="30%">
						<div className="flex flex-col h-full">
							{selectedFile ? (
								<>
									<Breadcrumb segments={breadcrumbSegments}>
										{selectedFile && (
											<FileCommentPopoverTrigger
												comments={allComments}
												filePath={selectedFile}
												onAdd={handleAddFileComment}
												onAppend={appendComment}
												onResolve={resolveThread}
												onDelete={deleteThread}
												open={openFileCommentsForFile === selectedFile}
												onOpenChange={(o) => {
													if (o && selectedFile) {
														setOpenFileCommentsForFile(selectedFile);
													} else {
														setOpenFileCommentsForFile(null);
													}
												}}
											/>
										)}
									</Breadcrumb>
									{isMarkdown && (
										<div className="flex items-center justify-end px-2 h-[28px] border-b border-border bg-card shrink-0">
											<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
												<Tooltip>
													<TooltipTrigger asChild>
														<Button
															variant="ghost"
															size="icon-xs"
															aria-label="Code"
															aria-pressed={!showMarkdownPreview}
															onClick={() => setShowMarkdownPreview(false)}
															className={cn(
																"w-6 h-5",
																!showMarkdownPreview
																	? "bg-background shadow-sm text-foreground"
																	: "text-muted-foreground hover:text-foreground",
															)}
														>
															<Code className="h-3.5 w-3.5" />
														</Button>
													</TooltipTrigger>
													<TooltipContent side="bottom" className="text-xs">
														Code
													</TooltipContent>
												</Tooltip>
												<Tooltip>
													<TooltipTrigger asChild>
														<Button
															variant="ghost"
															size="icon-xs"
															aria-label="Preview"
															aria-pressed={showMarkdownPreview}
															onClick={() => setShowMarkdownPreview(true)}
															className={cn(
																"w-6 h-5",
																showMarkdownPreview
																	? "bg-background shadow-sm text-foreground"
																	: "text-muted-foreground hover:text-foreground",
															)}
														>
															<Eye className="h-3.5 w-3.5" />
														</Button>
													</TooltipTrigger>
													<TooltipContent side="bottom" className="text-xs">
														Preview
													</TooltipContent>
												</Tooltip>
											</div>
										</div>
									)}
									<div className="flex-1 min-h-0 overflow-hidden">
										<DiffViewerSection
											isImage={isImage}
											isMarkdown={isMarkdown}
											showPreview={isMarkdown && showMarkdownPreview}
											imageDiff={imageDiff}
											binaryView={binaryView}
											fallbackView={fallbackView}
											error={reviewFileViewError}
											originalContent={originalContent}
											modifiedContent={modifiedContent}
											diffMode={diffMode}
											diffOnlyMode={diffOnlyMode}
											filePath={selectedFile}
											hunks={isTextDiff ? hunks : null}
											changeGroups={
												canUseGroupActions
													? (changeGroups ?? undefined)
													: undefined
											}
											onStageGroup={
												canUseGroupActions ? handleGroupAction : undefined
											}
											groupActionLabel={
												canUseGroupActions ? groupActionLabel : undefined
											}
											comments={lineComments}
											onAddComment={handleAddLineComment}
											onAddRangeComment={handleAddRangeComment}
											onAppendComment={appendComment}
											onResolveThread={resolveThread}
											onDeleteThread={deleteThread}
											scrollToLine={scrollToLine}
											scrollToThread={scrollToThread}
											onLineRangeSelected={handleLineRangeSelected}
										/>
									</div>
									<DiffToolbar
										diffMode={diffMode}
										diffOnlyMode={diffOnlyMode}
										onDiffModeChange={setDiffMode}
										onDiffOnlyModeChange={onDiffOnlyModeChange}
										fileNavigation={fileNavigation}
										onGoToPrevFile={handleGoToPrevFile}
										onGoToNextFile={handleGoToNextFile}
										filePath={selectedFilePath}
									/>
								</>
							) : (
								<div className="flex-1 flex items-center justify-center text-muted-foreground text-xs">
									Select a file to view diff
								</div>
							)}
						</div>
					</Panel>
				</Group>
			</div>
		</div>
	);
}
