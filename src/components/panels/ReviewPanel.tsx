import {
	Code,
	Eye,
	GitBranch,
	GitCommitHorizontal,
	PanelLeftClose,
	PanelLeftOpen,
} from "lucide-react";
import { useCallback, useRef, useState } from "react";
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
import { useDiffFileTree } from "@/hooks/useDiffFileTree";
import { useFileDiffContent } from "@/hooks/useFileDiffContent";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitStatus } from "@/hooks/useGitStatus";
import { useHunks } from "@/hooks/useHunks";
import { useImageDiff } from "@/hooks/useImageDiff";
import { useReviewPanel } from "@/hooks/useReviewPanel";
import { isImageFile } from "@/lib/imageUtils";
import { isMarkdownFile } from "@/lib/markdownUtils";
import { cn } from "@/lib/utils";
import type { DiffBase, DiffMode, DiffSection } from "@/types/settings";
import { DiffFileTree } from "./DiffFileTree";
import { DiffToolbar } from "./DiffToolbar";
import { DiffViewerSection } from "./DiffViewerSection";
import { useDiffOperations } from "./useDiffOperations";

interface ReviewPanelProps {
	rootPath: string;
	baseBranch: string | null;
	defaultDiffBase?: DiffBase;
	defaultDiffMode?: DiffMode;
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

	// File diff content
	const selectedFilePath = selectedFile ? `${rootPath}/${selectedFile}` : null;
	const { originalContent, modifiedContent } = useFileDiffContent(
		selectedFilePath,
		diffBase,
		selectedSection,
		gitRefreshKey,
	);

	// Hunks
	const { changeGroups, currentIndex, total, goToNext, goToPrev } = useHunks(
		originalContent,
		modifiedContent,
		selectedFile ?? undefined,
	);

	// Image / Markdown detection
	const isImage = selectedFile ? isImageFile(selectedFile) : false;
	const isMarkdown = selectedFile ? isMarkdownFile(selectedFile) : false;
	const [showMarkdownPreview, setShowMarkdownPreview] = useState(true);
	const imageDiff = useImageDiff(
		isImage ? selectedFilePath : null,
		diffBase,
		selectedSection,
		gitRefreshKey,
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
			refreshAfterAction();
		},
		[rootPath, stage, refreshAfterAction],
	);

	const handleUnstageFile = useCallback(
		async (path: string) => {
			if (!rootPath) return;
			await unstage(rootPath, [path]);
			refreshAfterAction();
		},
		[rootPath, unstage, refreshAfterAction],
	);

	const handleStageAll = useCallback(async () => {
		if (!rootPath) return;
		const paths = changedFiles.map((f) => f.path);
		if (paths.length === 0) return;
		await stage(rootPath, paths);
		refreshAfterAction();
	}, [rootPath, changedFiles, stage, refreshAfterAction]);

	const handleUnstageAll = useCallback(async () => {
		if (!rootPath) return;
		const paths = stagedFiles.map((f) => f.path);
		if (paths.length === 0) return;
		await unstage(rootPath, paths);
		refreshAfterAction();
	}, [rootPath, stagedFiles, unstage, refreshAfterAction]);

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
					<DiffBaseToggle diffBase={diffBase} onDiffBaseChange={setDiffBase} />
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
				<DiffBaseToggle diffBase={diffBase} onDiffBaseChange={setDiffBase} />
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
									<div className="flex-1 overflow-auto">
										<DiffViewerSection
											isImage={isImage}
											isMarkdown={isMarkdown}
											showPreview={isMarkdown && showMarkdownPreview}
											imageDiff={imageDiff}
											originalContent={originalContent}
											modifiedContent={modifiedContent}
											diffMode={diffMode}
											filePath={selectedFile}
											changeGroups={isBranchBase ? undefined : changeGroups}
											onStageGroup={
												isBranchBase ? undefined : handleGroupAction
											}
											groupActionLabel={
												isBranchBase ? undefined : groupActionLabel
											}
										/>
									</div>
									<DiffToolbar
										diffMode={diffMode}
										currentIndex={currentIndex}
										total={total}
										onDiffModeChange={setDiffMode}
										onGoToPrev={goToPrev}
										onGoToNext={goToNext}
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
