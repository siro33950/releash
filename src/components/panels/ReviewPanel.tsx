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
import { useBranchDiffFiles } from "@/hooks/useBranchDiffFiles";
import { useDiffComments } from "@/hooks/useDiffComments";
import { useDiffFileTree } from "@/hooks/useDiffFileTree";
import { useFileDiffContent } from "@/hooks/useFileDiffContent";
import { useFileNavigation } from "@/hooks/useFileNavigation";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitStatus } from "@/hooks/useGitStatus";
import { useHunks } from "@/hooks/useHunks";
import { useImageDiff } from "@/hooks/useImageDiff";
import { useReviewPanel } from "@/hooks/useReviewPanel";
import { isImageFile } from "@/lib/imageUtils";
import { isMarkdownFile } from "@/lib/markdownUtils";
import { cn } from "@/lib/utils";
import type { DiffBase, DiffMode, DiffSection } from "@/types/settings";
import { Breadcrumb } from "./Breadcrumb";
import { FileCommentPopoverTrigger } from "./DiffFileComment";
import { DiffFileTree } from "./DiffFileTree";
import { DiffToolbar } from "./DiffToolbar";
import { DiffViewerSection } from "./DiffViewerSection";
import { useDiffOperations } from "./useDiffOperations";

interface ReviewPanelProps {
	rootPath: string;
	baseBranch: string | null;
	defaultDiffBase?: DiffBase;
	defaultDiffMode?: DiffMode;
	diffOnlyMode: boolean;
	onDiffOnlyModeChange: (enabled: boolean) => void;
	navigateToFile?: { path: string; line?: number } | null;
	onSendToAgent?: (message: string) => Promise<void>;
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
	baseBranch,
	defaultDiffBase,
	defaultDiffMode,
	diffOnlyMode,
	onDiffOnlyModeChange,
	navigateToFile,
	onSendToAgent,
}: ReviewPanelProps) {
	const {
		diffBase,
		diffMode,
		selectedFile,
		selectedSection,
		setDiffBase,
		setDiffMode,
		selectFile,
	} = useReviewPanel({
		initialDiffBase: defaultDiffBase,
		initialDiffMode: defaultDiffMode,
	});

	const [gitRefreshKey, setGitRefreshKey] = useState(0);

	// File lists
	const { files: branchDiffFiles } = useBranchDiffFiles(
		rootPath,
		diffBase === "branch-base",
		baseBranch,
	);
	const {
		stagedFiles,
		changedFiles,
		refresh: refreshGitStatus,
	} = useGitStatus(rootPath, gitRefreshKey);

	// Tree
	const {
		stagedTree,
		changesTree,
		stagedFileCount,
		changesFileCount,
		branchBaseTree,
		branchBaseFileCount,
	} = useDiffFileTree(
		diffBase,
		branchDiffFiles,
		stagedFiles,
		changedFiles,
		rootPath,
	);

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

	useEffect(() => {
		if (!navigateToFile) return;
		const section = determineSectionForFile(navigateToFile.path);
		selectFile(navigateToFile.path, section);
		setScrollToLine(navigateToFile.line ?? null);
	}, [navigateToFile, determineSectionForFile, selectFile]);

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
	const { originalContent, modifiedContent } = useFileDiffContent(
		selectedFilePath,
		diffBase,
		selectedSection,
		gitRefreshKey,
	);

	// Hunks
	const { changeGroups } = useHunks(
		originalContent,
		modifiedContent,
		selectedFile ?? undefined,
	);

	// Image / Markdown detection
	const isImage = selectedFile ? isImageFile(selectedFile) : false;
	const isMarkdown = selectedFile ? isMarkdownFile(selectedFile) : false;
	const [showMarkdownPreview, setShowMarkdownPreview] = useState(false);
	const imageDiff = useImageDiff(
		isImage ? selectedFilePath : null,
		diffBase,
		selectedSection,
		gitRefreshKey,
	);

	// Diff comments
	const worktreeName = useMemo(() => {
		const parts = rootPath.split("/");
		return parts[parts.length - 1] ?? "";
	}, [rootPath]);

	const {
		comments: allComments,
		addComment,
		updateComment,
		deleteComment,
		sendToAgent,
		markSent,
		getCommentsForFile,
	} = useDiffComments({ worktreeName });

	const fileComments = useMemo(
		() => (selectedFile ? getCommentsForFile(selectedFile) : []),
		[selectedFile, getCommentsForFile],
	);

	const lineComments = useMemo(
		() => fileComments.filter((c) => c.lineNumber != null),
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

	const handleSendComments = useCallback(
		async (commentIds: string[]) => {
			const result = await sendToAgent(commentIds);
			if (result.formattedMessage && onSendToAgent) {
				await onSendToAgent(result.formattedMessage);
				await markSent(result.commentIds);
			}
		},
		[sendToAgent, markSent, onSendToAgent],
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
	const { stage, unstage, stageHunk, unstageHunk } = useGitActions();

	const refreshAfterAction = useCallback(() => {
		setGitRefreshKey((k) => k + 1);
		refreshGitStatus();
	}, [refreshGitStatus]);

	// Determine action label and handler based on section
	const isBranchBase = diffBase === "branch-base";
	const groupActionLabel = selectedSection === "staged" ? "Unstage" : "Stage";

	const diffOps = useDiffOperations({
		filePath: selectedFilePath ?? "",
		rootPath,
		originalContent,
		modifiedContent,
		onStageHunk: stageHunk,
		onUnstageHunk: unstageHunk,
		onGitChanged: refreshAfterAction,
	});

	const handleGroupAction =
		selectedSection === "staged"
			? diffOps.handleUnstageGroup
			: diffOps.handleStageGroup;

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

	// Empty state
	if (totalFileCount === 0) {
		return (
			<div className="flex flex-col h-full">
				<div className="flex items-center justify-between px-2 h-[32px] border-b border-border bg-card shrink-0">
					<div className="w-5" />
					<div className="flex items-center gap-1">
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
						defaultSize="30%"
						minSize="15%"
						collapsible
						onResize={(size) => setFileTreeCollapsed(size.asPercentage <= 0)}
					>
						<div className="h-full overflow-hidden border-r border-border">
							<DiffFileTree
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
												onUpdate={updateComment}
												onDelete={deleteComment}
												onSend={handleSendComments}
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
											originalContent={originalContent}
											modifiedContent={modifiedContent}
											diffMode={diffMode}
											diffOnlyMode={diffOnlyMode}
											filePath={selectedFile}
											changeGroups={isBranchBase ? undefined : changeGroups}
											onStageGroup={
												isBranchBase ? undefined : handleGroupAction
											}
											groupActionLabel={
												isBranchBase ? undefined : groupActionLabel
											}
											comments={lineComments}
											onAddComment={handleAddLineComment}
											onAddRangeComment={handleAddRangeComment}
											onUpdateComment={updateComment}
											onDeleteComment={deleteComment}
											onSendComment={handleSendComments}
											scrollToLine={scrollToLine}
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
