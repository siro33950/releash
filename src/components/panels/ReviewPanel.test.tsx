import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { invokeClient, listenClient } from "@/lib/client";
import type { GitFileStatus } from "@/types/git";
import type { DiffTreeNode, ReviewFileView } from "@/types/review";
import type { DiffViewerSectionProps } from "./DiffViewerSection";
import { ReviewPanel } from "./ReviewPanel";

// Radix UI Tooltip require pointer capture APIs not available in jsdom
if (typeof Element.prototype.hasPointerCapture !== "function") {
	Element.prototype.hasPointerCapture = () => false;
}
if (typeof Element.prototype.setPointerCapture !== "function") {
	Element.prototype.setPointerCapture = () => {};
}
if (typeof Element.prototype.releasePointerCapture !== "function") {
	Element.prototype.releasePointerCapture = () => {};
}

vi.mock("@react-symbols/icons/utils", () => ({
	FileIcon: ({ fileName }: { fileName: string }) => (
		<span data-testid="file-icon" data-filename={fileName} />
	),
	FolderIcon: ({ folderName }: { folderName: string }) => (
		<span data-testid="folder-icon" data-foldername={folderName} />
	),
}));

// react-resizable-panels does not work in jsdom
vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => (
		<div data-testid="resizable-group">{children}</div>
	),
	Panel: ({
		children,
		id,
	}: {
		children: React.ReactNode;
		id?: string;
		[key: string]: unknown;
	}) => <div data-testid={`panel-${id ?? "unknown"}`}>{children}</div>,
	Separator: () => <div data-testid="separator" />,
}));

vi.mock("@/hooks/useReviewPanel", () => ({
	useReviewPanel: vi.fn().mockReturnValue({
		diffBase: "head",
		diffMode: "gutter",
		selectedFile: null,
		selectedSection: "changes",
		setDiffBase: vi.fn(),
		setDiffMode: vi.fn(),
		selectFile: vi.fn(),
	}),
}));

vi.mock("@/hooks/useReviewSnapshot", () => ({
	useReviewSnapshot: vi.fn().mockReturnValue({
		files: [],
		stagedFiles: [],
		changedFiles: [],
		stagedTree: [],
		changesTree: [],
		stagedFileCount: 0,
		changesFileCount: 0,
		branchBaseTree: [],
		branchBaseFileCount: 0,
		version: 0,
		loading: false,
		refresh: vi.fn(),
	}),
}));

vi.mock("@/hooks/useReviewFileView", () => ({
	useReviewFileView: vi.fn().mockReturnValue({
		view: null,
		originalContent: "",
		modifiedContent: "",
		hunks: [],
		changeGroups: [],
		imageDiff: { originalUrl: null, modifiedUrl: null, loading: false },
		loading: false,
		error: null,
	}),
}));

vi.mock("@/hooks/useGitActions", () => ({
	useGitActions: vi.fn().mockReturnValue({
		stage: vi.fn(),
		unstage: vi.fn(),
		createBranch: vi.fn(),
	}),
}));

vi.mock("@/hooks/useGitEventRefresh", () => ({
	useGitEventRefresh: vi.fn(),
}));

vi.mock("@/hooks/useFileNavigation", () => ({
	useFileNavigation: vi.fn().mockReturnValue({
		fileNavigation: {
			current_index: 0,
			total: 0,
			prev_file: null,
			next_file: null,
		},
		goToPrevFile: vi.fn(),
		goToNextFile: vi.fn(),
	}),
}));

vi.mock("@/lib/client", () => ({
	listenClient: vi.fn().mockResolvedValue(() => {}),
	invokeClient: vi.fn().mockResolvedValue(null),
}));

vi.mock("./DiffViewerSection", () => ({
	DiffViewerSection: ({
		error,
		isMarkdown,
		showPreview,
		changeGroups,
		onStageGroup,
		groupActionLabel,
	}: DiffViewerSectionProps) => (
		<div
			data-testid="diff-viewer-section"
			data-error={error ?? ""}
			data-is-markdown={String(Boolean(isMarkdown))}
			data-show-preview={String(Boolean(showPreview))}
		>
			{changeGroups?.map((group) => (
				<button
					type="button"
					key={group.groupId}
					disabled={!onStageGroup}
					onClick={() => onStageGroup?.(group.groupId)}
				>
					{groupActionLabel} hunk
				</button>
			))}
		</div>
	),
}));

vi.mock("./DiffToolbar", () => ({
	DiffToolbar: ({ filePath }: { filePath?: string | null }) => (
		<div data-testid="diff-toolbar" data-file-path={filePath ?? ""} />
	),
}));

const { useReviewSnapshot } = await import("@/hooks/useReviewSnapshot");
const { useReviewFileView } = await import("@/hooks/useReviewFileView");
const { useReviewPanel } = await import("@/hooks/useReviewPanel");
const { useGitActions } = await import("@/hooks/useGitActions");
const { useGitEventRefresh } = await import("@/hooks/useGitEventRefresh");

function mockReviewSnapshot(
	overrides: Partial<ReturnType<typeof useReviewSnapshot>>,
) {
	vi.mocked(useReviewSnapshot).mockReturnValue({
		files: [],
		stagedFiles: [],
		changedFiles: [],
		stagedTree: [],
		changesTree: [],
		stagedFileCount: 0,
		changesFileCount: 0,
		branchBaseTree: [],
		branchBaseFileCount: 0,
		version: 0,
		loading: false,
		refresh: vi.fn(),
		snapshot: {
			version: 0,
			stale: false,
			loading: false,
			base: "head",
			files: [],
			stagedFiles: [],
			changedFiles: [],
			diffStats: [],
			tree: [],
			stagedTree: [],
			changesTree: [],
			stagedFileCount: 0,
			changesFileCount: 0,
		},
		...overrides,
	});
}

function makeTextDiffView(
	path: string,
): Extract<ReviewFileView, { kind: "textDiff" }> {
	return {
		kind: "textDiff",
		version: 0,
		stale: false,
		fileId: path,
		path,
		original: "",
		modified: "",
		source: "diff",
		hunks: [],
		changeGroups: [],
		limited: false,
		viewport: null,
		totalLines: 0,
	};
}

function makeBinaryView(path: string): ReviewFileView {
	return {
		kind: "binary",
		version: 0,
		stale: false,
		fileId: path,
		path,
		originalUrl: null,
		modifiedUrl: null,
		originalSize: null,
		modifiedSize: null,
	};
}

function mockSelectedReviewFile(path: string, view: ReviewFileView) {
	vi.mocked(useReviewPanel).mockReturnValue({
		diffBase: "head",
		diffMode: "gutter",
		selectedFile: path,
		selectedSection: "changes",
		setDiffBase: vi.fn(),
		setDiffMode: vi.fn(),
		selectFile: vi.fn(),
	});
	mockReviewSnapshot({
		stagedTree: [],
		changesTree: [
			{
				id: `file:${path}`,
				name: path.split("/").pop() ?? path,
				path,
				node_type: "file",
				status: "modified",
				additions: 1,
				deletions: 0,
				children: [],
			},
		],
		stagedFileCount: 0,
		changesFileCount: 1,
		branchBaseTree: [],
		branchBaseFileCount: 0,
		loading: false,
	});
	vi.mocked(useReviewFileView).mockReturnValue({
		view,
		originalContent: view.kind === "textDiff" ? view.original : "",
		modifiedContent: view.kind === "textDiff" ? view.modified : "",
		hunks: view.kind === "textDiff" ? view.hunks : null,
		changeGroups: view.kind === "textDiff" ? view.changeGroups : null,
		imageDiff: { originalUrl: null, modifiedUrl: null, loading: false },
		loading: false,
		error: null,
	});
}

function gitStatus(
	path: string,
	indexStatus: GitFileStatus["index_status"],
	worktreeStatus: GitFileStatus["worktree_status"],
): GitFileStatus {
	return {
		path,
		index_status: indexStatus,
		worktree_status: worktreeStatus,
	};
}

function treeFile(path: string, status: string): DiffTreeNode {
	return {
		id: `file:${path}`,
		name: path,
		path,
		node_type: "file",
		status,
		additions: 1,
		deletions: 0,
		children: [],
	};
}

function mockNonEmptyHeadSnapshot(
	overrides: Partial<ReturnType<typeof useReviewSnapshot>> = {},
) {
	const stagedOnly = gitStatus("staged-only.ts", "modified", "none");
	const changedOnly = gitStatus("changed-only.ts", "none", "modified");
	const both = gitStatus("both.ts", "modified", "modified");

	mockReviewSnapshot({
		stagedFiles: [stagedOnly, both],
		changedFiles: [changedOnly, both],
		stagedTree: [
			treeFile(stagedOnly.path, stagedOnly.index_status),
			treeFile(both.path, both.index_status),
		],
		changesTree: [
			treeFile(changedOnly.path, changedOnly.worktree_status),
			treeFile(both.path, both.worktree_status),
		],
		stagedFileCount: 2,
		changesFileCount: 2,
		branchBaseTree: [],
		branchBaseFileCount: 0,
		loading: false,
		...overrides,
	});
}

describe("ReviewPanel", () => {
	it("再接続時のコメント取得失敗を空のreviewにも表示する", async () => {
		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);
		const reconnect = vi
			.mocked(listenClient)
			.mock.calls.find(([event]) => event === "review-comments-changed")?.[2];
		expect(reconnect).toBeDefined();
		vi.mocked(invokeClient).mockRejectedValueOnce(new Error("read denied"));
		await act(async () => reconnect?.());
		expect(screen.getByRole("alert")).toHaveTextContent(
			"コメントを取得できません: read denied",
		);
	});

	it("should show 'No changes' when totalFileCount is 0", () => {
		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByText("No changes")).toBeInTheDocument();
	});

	it("should show 'Select a file to view diff' when no file is selected and files exist", () => {
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByText("Select a file to view diff")).toBeInTheDocument();
	});

	it("should render DiffFileTree when files exist", () => {
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		// DiffFileTree is rendered inside the diff-files panel
		expect(screen.getByTestId("panel-diff-files")).toBeInTheDocument();
		expect(screen.getByTestId("panel-diff-view")).toBeInTheDocument();
	});

	it("classifies non-empty staged and changed collections and stages each supplied path list", async () => {
		const stage = vi.fn().mockResolvedValue(undefined);
		const unstage = vi.fn().mockResolvedValue(undefined);
		const createBranch = vi.fn().mockResolvedValue(undefined);
		vi.mocked(useGitActions).mockReturnValue({ stage, unstage, createBranch });
		mockNonEmptyHeadSnapshot();

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		const diffTree = screen.getByTestId("diff-file-tree");
		const [changesSection, stagedSection] = Array.from(
			diffTree.children,
		) as HTMLElement[];

		expect(within(changesSection).getByText("Unstaged")).toBeInTheDocument();
		expect(
			within(changesSection).getByText("changed-only.ts"),
		).toBeInTheDocument();
		expect(within(changesSection).getByText("both.ts")).toBeInTheDocument();
		expect(
			within(changesSection).queryByText("staged-only.ts"),
		).not.toBeInTheDocument();

		expect(within(stagedSection).getByText("Staged")).toBeInTheDocument();
		expect(
			within(stagedSection).getByText("staged-only.ts"),
		).toBeInTheDocument();
		expect(within(stagedSection).getByText("both.ts")).toBeInTheDocument();
		expect(
			within(stagedSection).queryByText("changed-only.ts"),
		).not.toBeInTheDocument();

		fireEvent.click(screen.getByRole("button", { name: "Stage All" }));
		await waitFor(() => {
			expect(stage).toHaveBeenCalledWith("/repo", [
				"changed-only.ts",
				"both.ts",
			]);
		});

		fireEvent.click(screen.getByRole("button", { name: "Unstage All" }));
		await waitFor(() => {
			expect(unstage).toHaveBeenCalledWith("/repo", [
				"staged-only.ts",
				"both.ts",
			]);
		});
	});

	it("selects review thread sections from supplied staged and changed memberships", async () => {
		const selectFile = vi.fn();
		mockNonEmptyHeadSnapshot({
			stagedTree: [],
			changesTree: [],
			stagedFileCount: 0,
			changesFileCount: 0,
		});
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: null,
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile,
		});

		const view = render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
					navigateToThread={{
						filePath: "staged-only.ts",
						threadId: "thread-staged",
						isFileComment: true,
					}}
				/>
			</TooltipProvider>,
		);

		await waitFor(() => {
			expect(selectFile).toHaveBeenCalledWith("staged-only.ts", "staged");
		});

		selectFile.mockClear();
		view.rerender(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
					navigateToThread={{
						filePath: "changed-only.ts",
						threadId: "thread-changed",
						isFileComment: true,
					}}
				/>
			</TooltipProvider>,
		);

		await waitFor(() => {
			expect(selectFile).toHaveBeenCalledWith("changed-only.ts", "changes");
		});

		selectFile.mockClear();
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: null,
			selectedSection: "staged",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile,
		});
		view.rerender(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
					navigateToThread={{
						filePath: "both.ts",
						threadId: "thread-both",
						isFileComment: true,
					}}
				/>
			</TooltipProvider>,
		);

		await waitFor(() => {
			expect(selectFile).toHaveBeenCalledWith("both.ts", "staged");
		});
	});

	it("should show breadcrumb when a file is selected", () => {
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: "src/components/App.tsx",
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:src/components/App.tsx",
					name: "App.tsx",
					path: "src/components/App.tsx",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByTestId("breadcrumb")).toBeInTheDocument();
		const breadcrumb = within(screen.getByTestId("breadcrumb"));
		expect(breadcrumb.getByText("src")).toBeInTheDocument();
		expect(breadcrumb.getByText("components")).toBeInTheDocument();
		expect(breadcrumb.getByText("App.tsx")).toBeInTheDocument();
	});

	it("passes review file view errors to the diff viewer", () => {
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: "src/main.ts",
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:src/main.ts",
					name: "main.ts",
					path: "src/main.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});
		vi.mocked(useReviewFileView).mockReturnValue({
			view: null,
			originalContent: "",
			modifiedContent: "",
			hunks: null,
			changeGroups: null,
			imageDiff: { originalUrl: null, modifiedUrl: null, loading: false },
			loading: false,
			error: "Failed to load diff: review target is not in snapshot",
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByTestId("diff-viewer-section")).toHaveAttribute(
			"data-error",
			"Failed to load diff: review target is not in snapshot",
		);
	});

	it("shows markdown preview toggle for selected markdown text diff", async () => {
		const user = userEvent.setup();
		mockSelectedReviewFile("README.md", makeTextDiffView("README.md"));

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByRole("button", { name: "Code" })).toBeInTheDocument();
		const previewButton = screen.getByRole("button", { name: "Preview" });
		expect(previewButton).toBeInTheDocument();
		expect(screen.getByTestId("diff-viewer-section")).toHaveAttribute(
			"data-is-markdown",
			"true",
		);
		expect(screen.getByTestId("diff-viewer-section")).toHaveAttribute(
			"data-show-preview",
			"false",
		);

		await user.click(previewButton);

		expect(screen.getByTestId("diff-viewer-section")).toHaveAttribute(
			"data-show-preview",
			"true",
		);
	});

	it("does not show markdown preview toggle for non-markdown text diff", () => {
		mockSelectedReviewFile("src/main.rs", makeTextDiffView("src/main.rs"));

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
				/>
			</TooltipProvider>,
		);

		expect(
			screen.queryByRole("button", { name: "Preview" }),
		).not.toBeInTheDocument();
		expect(screen.getByTestId("diff-viewer-section")).toHaveAttribute(
			"data-is-markdown",
			"false",
		);
	});

	it("does not show markdown preview toggle for markdown binary view", () => {
		mockSelectedReviewFile("README.md", makeBinaryView("README.md"));

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
				/>
			</TooltipProvider>,
		);

		expect(
			screen.queryByRole("button", { name: "Preview" }),
		).not.toBeInTheDocument();
		expect(screen.getByTestId("diff-viewer-section")).toHaveAttribute(
			"data-is-markdown",
			"false",
		);
	});

	it("should not show breadcrumb when no file is selected", () => {
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: null,
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
				/>
			</TooltipProvider>,
		);

		expect(screen.queryByTestId("breadcrumb")).not.toBeInTheDocument();
	});

	it("should pass null filePath to DiffToolbar when no file is selected", () => {
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		// No file selected → DiffToolbar is not rendered (placeholder shown instead)
		expect(screen.queryByTestId("diff-toolbar")).not.toBeInTheDocument();
	});

	it("clears agent editor line context when selected file is cleared externally", async () => {
		const onLineRangeSelected = vi.fn();
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:src/main.ts",
					name: "main.ts",
					path: "src/main.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: "src/main.ts",
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});

		const view = render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
					onLineRangeSelected={onLineRangeSelected}
				/>
			</TooltipProvider>,
		);
		expect(onLineRangeSelected).not.toHaveBeenCalled();

		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: null,
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});
		view.rerender(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={() => {}}
					onLineRangeSelected={onLineRangeSelected}
				/>
			</TooltipProvider>,
		);

		await waitFor(() =>
			expect(onLineRangeSelected).toHaveBeenCalledWith("", 0, 0),
		);
	});

	it("should pass filePath to DiffToolbar when a file is selected", () => {
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: "src/main.ts",
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});

		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:src/main.ts",
					name: "main.ts",
					path: "src/main.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		const toolbar = screen.getByTestId("diff-toolbar");
		expect(toolbar).toHaveAttribute("data-file-path", "/repo/src/main.ts");
	});

	it("should show 'Open in editor' button instead of 'Send all comments' button", () => {
		mockReviewSnapshot({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		expect(
			screen.getByRole("button", { name: "Open in editor" }),
		).toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Send all comments to Agent" }),
		).not.toBeInTheDocument();
	});

	describe("useGitEventRefresh integration", () => {
		it("should pass rootPath and callback to useGitEventRefresh", () => {
			render(
				<TooltipProvider>
					<ReviewPanel
						rootPath="/repo"
						diffOnlyMode={false}
						onDiffOnlyModeChange={vi.fn()}
					/>
				</TooltipProvider>,
			);

			expect(vi.mocked(useGitEventRefresh)).toHaveBeenCalledWith(
				"/repo",
				expect.any(Function),
			);
		});

		it("should increment gitRefreshKey when refresh callback is invoked", async () => {
			let capturedRefresh: (() => void) | undefined;
			vi.mocked(useGitEventRefresh).mockImplementation(
				(_rootPath, onRefresh) => {
					capturedRefresh = onRefresh;
				},
			);

			const refreshKeys: number[] = [];
			vi.mocked(useReviewFileView).mockImplementation(((
				_rootPath,
				_filePath,
				_diffBase,
				_section,
				gitRefreshKey,
			) => {
				refreshKeys.push(gitRefreshKey);
				return {
					view: null,
					originalContent: "",
					modifiedContent: "",
					hunks: [],
					changeGroups: [],
					imageDiff: { originalUrl: null, modifiedUrl: null, loading: false },
					loading: false,
					error: null,
				};
			}) as typeof useReviewFileView);

			render(
				<TooltipProvider>
					<ReviewPanel
						rootPath="/repo"
						diffOnlyMode={false}
						onDiffOnlyModeChange={vi.fn()}
					/>
				</TooltipProvider>,
			);

			expect(capturedRefresh).toBeDefined();

			await act(async () => {
				capturedRefresh?.();
			});

			expect(refreshKeys[refreshKeys.length - 1]).toBe(1);
		});

		it("should increment gitRefreshKey cumulatively on multiple events", async () => {
			let capturedRefresh: (() => void) | undefined;
			vi.mocked(useGitEventRefresh).mockImplementation(
				(_rootPath, onRefresh) => {
					capturedRefresh = onRefresh;
				},
			);

			const refreshKeys: number[] = [];
			vi.mocked(useReviewFileView).mockImplementation(((
				_rootPath,
				_filePath,
				_diffBase,
				_section,
				gitRefreshKey,
			) => {
				refreshKeys.push(gitRefreshKey);
				return {
					view: null,
					originalContent: "",
					modifiedContent: "",
					hunks: [],
					changeGroups: [],
					imageDiff: { originalUrl: null, modifiedUrl: null, loading: false },
					loading: false,
					error: null,
				};
			}) as typeof useReviewFileView);

			render(
				<TooltipProvider>
					<ReviewPanel
						rootPath="/repo"
						diffOnlyMode={false}
						onDiffOnlyModeChange={vi.fn()}
					/>
				</TooltipProvider>,
			);

			expect(capturedRefresh).toBeDefined();

			await act(async () => {
				capturedRefresh?.();
			});
			await act(async () => {
				capturedRefresh?.();
			});

			expect(refreshKeys[refreshKeys.length - 1]).toBe(2);
		});
	});
	it.each(["changes", "staged"] as const)(
		"staleな差分では%sのhunk操作を表示しない",
		(section) => {
			mockSelectedReviewFile("both.ts", {
				...makeTextDiffView("both.ts"),
				version: 1,
				stale: true,
				changeGroups: [
					{
						groupIndex: 0,
						groupId: "group:both.ts:0",
						hunkIndex: 0,
						newStart: 1,
						newEnd: 1,
						lineOffsetStart: 0,
						lineOffsetEnd: 1,
					},
				],
			});
			mockNonEmptyHeadSnapshot({ version: 10 });
			vi.mocked(useReviewPanel).mockReturnValue({
				diffBase: "head",
				diffMode: "gutter",
				selectedFile: "both.ts",
				selectedSection: section,
				setDiffBase: vi.fn(),
				setDiffMode: vi.fn(),
				selectFile: vi.fn(),
			});

			render(
				<TooltipProvider>
					<ReviewPanel
						rootPath="/repo"
						diffOnlyMode={false}
						onDiffOnlyModeChange={vi.fn()}
					/>
				</TooltipProvider>,
			);

			expect(
				within(screen.getByTestId("diff-viewer-section")).queryByRole(
					"button",
					{ name: /hunk/ },
				),
			).not.toBeInTheDocument();
		},
	);
	it.each(["changes", "staged"] as const)(
		"番号の振り直し後に%sのhunk操作を実行できる",
		async (section) => {
			const actualSnapshot = await vi.importActual<
				typeof import("@/hooks/useReviewSnapshot")
			>("@/hooks/useReviewSnapshot");
			const actualFileView = await vi.importActual<
				typeof import("@/hooks/useReviewFileView")
			>("@/hooks/useReviewFileView");
			mockSelectedReviewFile("file.ts", makeTextDiffView("file.ts"));
			const panelState = vi.mocked(useReviewPanel).getMockImplementation()?.(
				{},
			);
			if (!panelState) throw new Error("missing panel state");
			vi.mocked(useReviewPanel).mockReturnValue({
				...panelState,
				selectedSection: section,
			});
			const initialSnapshot = vi
				.mocked(useReviewSnapshot)
				.getMockImplementation()?.("/repo", "head").snapshot;
			if (!initialSnapshot) throw new Error("missing snapshot");
			vi.mocked(useReviewSnapshot).mockImplementation(
				actualSnapshot.useReviewSnapshot,
			);
			vi.mocked(useReviewFileView).mockImplementation(
				actualFileView.useReviewFileView,
			);
			let version = 10;
			const groupId = "group:file.ts:0";
			vi.mocked(invokeClient).mockClear();
			vi.mocked(invokeClient).mockImplementation(async (command, args) => {
				if (command === "get_review_snapshot")
					return {
						...initialSnapshot,
						version,
						changesTree: [treeFile("file.ts", "modified")],
						changesFileCount: 1,
					};
				if (command === "get_review_file_view")
					return {
						...makeTextDiffView("file.ts"),
						version,
						stale:
							(args as { input: { snapshotVersion: number } }).input
								.snapshotVersion !== version,
						changeGroups: [
							{
								groupIndex: 0,
								groupId,
								hunkIndex: 0,
								newStart: 1,
								newEnd: 1,
								lineOffsetStart: 0,
								lineOffsetEnd: 1,
							},
						],
					};
				return null;
			});
			const { unmount } = render(
				<TooltipProvider>
					<ReviewPanel
						rootPath="/repo"
						diffOnlyMode={false}
						onDiffOnlyModeChange={vi.fn()}
					/>
				</TooltipProvider>,
			);
			const label = section === "staged" ? "Unstage" : "Stage";
			await waitFor(() =>
				expect(
					screen.getByRole("button", { name: `${label} hunk` }),
				).toBeEnabled(),
			);

			version = 1;
			const refresh = vi.mocked(useGitEventRefresh).mock.lastCall?.[1];
			expect(refresh).toBeDefined();
			await act(async () => refresh?.());

			await waitFor(() =>
				expect(invokeClient).toHaveBeenCalledWith("get_review_file_view", {
					input: {
						worktreePath: "/repo",
						target: { by: "path", value: "file.ts" },
						section,
						base: "head",
						snapshotVersion: 1,
						viewport: null,
					},
				}),
			);
			await waitFor(() =>
				expect(
					screen.getByRole("button", { name: `${label} hunk` }),
				).toBeEnabled(),
			);
			fireEvent.click(screen.getByRole("button", { name: `${label} hunk` }));
			await waitFor(() =>
				expect(invokeClient).toHaveBeenCalledWith(
					section === "staged"
						? "git_unstage_review_group"
						: "git_stage_review_group",
					{
						input: {
							worktreePath: "/repo",
							path: "file.ts",
							section,
							base: "head",
							groupId,
						},
					},
				),
			);
			unmount();
			mockReviewSnapshot({});
			vi.mocked(invokeClient).mockReset().mockResolvedValue(null);
		},
	);
});
