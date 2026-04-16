import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useBaseBranch } from "@/hooks/useBaseBranch";
import type { BranchDiffChangedFile } from "@/hooks/useBranchDiffFiles";
import type { GitFileStatus } from "@/types/git";
import { SourceControlPanel } from "./SourceControlPanel";

const mockGitStatus = {
	statusMap: new Map(),
	stagedFiles: [] as GitFileStatus[],
	changedFiles: [] as GitFileStatus[],
	refresh: vi.fn(),
};

const mockGitActions = {
	stage: vi.fn().mockResolvedValue(undefined),
	unstage: vi.fn().mockResolvedValue(undefined),
	discard: vi.fn().mockResolvedValue(undefined),
	commit: vi.fn().mockResolvedValue("abc123"),
	push: vi.fn().mockResolvedValue("ok"),
	createBranch: vi.fn().mockResolvedValue(undefined),
	switchBranch: vi.fn().mockResolvedValue(undefined),
};

const mockEditorContext = {
	gitRefreshKey: 0,
	diffBase: "staged" as string,
	setDiffBase: vi.fn(),
};

const mockBranchDiffFiles = {
	files: [] as BranchDiffChangedFile[],
	loading: false,
	error: null as string | null,
	refresh: vi.fn(),
};

vi.mock("@/contexts/GitStatusContext", () => ({
	useGitStatusContext: () => mockGitStatus,
}));

vi.mock("@/contexts/EditorContext", () => ({
	useEditorContext: () => mockEditorContext,
}));

vi.mock("@/hooks/useGitActions", () => ({
	useGitActions: () => mockGitActions,
}));

vi.mock("@/hooks/useCurrentBranch", () => ({
	useCurrentBranch: () => ({ branch: "feature/test", refresh: vi.fn() }),
}));

vi.mock("@/hooks/useBaseBranch", () => ({
	useBaseBranch: vi.fn().mockReturnValue({
		baseBranch: "main",
		setBaseBranch: vi.fn(),
		localBranches: ["main", "develop"],
	}),
}));

vi.mock("@/hooks/useBranchDiffFiles", () => ({
	useBranchDiffFiles: () => mockBranchDiffFiles,
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
	revealItemInDir: vi.fn().mockResolvedValue(undefined),
}));

// Radix UI Select requires pointer capture APIs not available in jsdom
if (typeof Element.prototype.hasPointerCapture !== "function") {
	Element.prototype.hasPointerCapture = () => false;
}
if (typeof Element.prototype.setPointerCapture !== "function") {
	Element.prototype.setPointerCapture = () => {};
}
if (typeof Element.prototype.releasePointerCapture !== "function") {
	Element.prototype.releasePointerCapture = () => {};
}
if (typeof Element.prototype.scrollIntoView !== "function") {
	Element.prototype.scrollIntoView = () => {};
}

describe("SourceControlPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockGitStatus.stagedFiles = [];
		mockGitStatus.changedFiles = [];
		mockEditorContext.diffBase = "staged";
		mockBranchDiffFiles.files = [];
		mockBranchDiffFiles.loading = false;
		mockBranchDiffFiles.error = null;
		mockGitActions.stage.mockResolvedValue(undefined);
		mockGitActions.unstage.mockResolvedValue(undefined);
		mockGitActions.discard.mockResolvedValue(undefined);
		mockGitActions.commit.mockResolvedValue("abc123");
		mockGitActions.push.mockResolvedValue("ok");
		vi.mocked(useBaseBranch).mockReturnValue({
			baseBranch: "main",
			setBaseBranch: vi.fn(),
			localBranches: ["main", "develop"],
		});
	});

	it("should show message when no folder is opened", () => {
		render(<SourceControlPanel rootPath={null} />);
		expect(screen.getByText("No folder opened")).toBeInTheDocument();
	});

	it("should show 'No changes' when there are no files", () => {
		render(<SourceControlPanel rootPath="/test/repo" />);
		expect(screen.getByText("No changes")).toBeInTheDocument();
	});

	it("should show header with file count", () => {
		mockGitStatus.changedFiles = [
			{
				path: "src/file.txt",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);
		expect(screen.getByText("1 file changes")).toBeInTheDocument();
	});

	it("should show unstaged files section", () => {
		mockGitStatus.changedFiles = [
			{
				path: "src/modified.txt",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);
		expect(screen.getByText(/Unstaged Files/)).toBeInTheDocument();
		expect(screen.getByText("modified.txt")).toBeInTheDocument();
	});

	it("should show staged files section", () => {
		mockGitStatus.stagedFiles = [
			{
				path: "src/staged.txt",
				index_status: "new",
				worktree_status: "none",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);
		expect(screen.getByText(/Staged Files/)).toBeInTheDocument();
		expect(screen.getByText("staged.txt")).toBeInTheDocument();
	});

	it("should call stage on individual file action", async () => {
		mockGitStatus.changedFiles = [
			{
				path: "file.txt",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		const stageButton = screen.getByTitle("Stage");
		fireEvent.click(stageButton);

		await waitFor(() => {
			expect(mockGitActions.stage).toHaveBeenCalledWith("/test/repo", [
				"file.txt",
			]);
		});
		expect(mockGitStatus.refresh).toHaveBeenCalled();
	});

	it("should call unstage on individual file action", async () => {
		mockGitStatus.stagedFiles = [
			{
				path: "file.txt",
				index_status: "new",
				worktree_status: "none",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		const unstageButton = screen.getByTitle("Unstage");
		fireEvent.click(unstageButton);

		await waitFor(() => {
			expect(mockGitActions.unstage).toHaveBeenCalledWith("/test/repo", [
				"file.txt",
			]);
		});
		expect(mockGitStatus.refresh).toHaveBeenCalled();
	});

	it("should call stage all on section action", async () => {
		mockGitStatus.changedFiles = [
			{
				path: "a.txt",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.click(screen.getByTitle("Stage All Changes"));

		await waitFor(() => {
			expect(mockGitActions.stage).toHaveBeenCalledWith("/test/repo", []);
		});
	});

	it("should call unstage all on section action", async () => {
		mockGitStatus.stagedFiles = [
			{
				path: "a.txt",
				index_status: "new",
				worktree_status: "none",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.click(screen.getByTitle("Unstage All Changes"));

		await waitFor(() => {
			expect(mockGitActions.unstage).toHaveBeenCalledWith("/test/repo", []);
		});
	});

	it("should disable commit when summary is empty", () => {
		mockGitStatus.stagedFiles = [
			{
				path: "file.txt",
				index_status: "new",
				worktree_status: "none",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		const commitButton = screen.getByText("Commit");
		expect(commitButton).toBeDisabled();
	});

	it("should disable commit when no staged files", () => {
		render(<SourceControlPanel rootPath="/test/repo" />);

		const summaryInput = screen.getByPlaceholderText("Commit summary");
		fireEvent.change(summaryInput, { target: { value: "test" } });

		const commitButton = screen.getByText("Commit");
		expect(commitButton).toBeDisabled();
	});

	it("should commit with summary and description", async () => {
		mockGitStatus.stagedFiles = [
			{
				path: "file.txt",
				index_status: "new",
				worktree_status: "none",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.change(screen.getByPlaceholderText("Commit summary"), {
			target: { value: "feat: test" },
		});
		fireEvent.change(screen.getByPlaceholderText("Description"), {
			target: { value: "details" },
		});
		fireEvent.click(screen.getByText("Commit"));

		await waitFor(() => {
			expect(mockGitActions.commit).toHaveBeenCalledWith(
				"/test/repo",
				"feat: test\n\ndetails",
			);
		});
		expect(mockGitStatus.refresh).toHaveBeenCalled();
	});

	it("should show error on commit failure", async () => {
		mockGitActions.commit.mockRejectedValue(new Error("commit failed"));
		mockGitStatus.stagedFiles = [
			{
				path: "file.txt",
				index_status: "new",
				worktree_status: "none",
			},
		];
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.change(screen.getByPlaceholderText("Commit summary"), {
			target: { value: "test" },
		});
		fireEvent.click(screen.getByText("Commit"));

		await waitFor(() => {
			expect(screen.getByText(/commit failed/)).toBeInTheDocument();
		});
	});

	it("should call push", async () => {
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.click(screen.getByText("Push"));

		await waitFor(() => {
			expect(mockGitActions.push).toHaveBeenCalledWith("/test/repo");
		});
	});

	it("should show 'Pushing...' while push is in progress", async () => {
		let resolvePush: ((value: unknown) => void) | undefined;
		mockGitActions.push.mockImplementation(
			() =>
				new Promise((resolve) => {
					resolvePush = resolve;
				}),
		);
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.click(screen.getByText("Push"));

		await waitFor(() => {
			expect(screen.getByText("Pushing...")).toBeInTheDocument();
		});

		resolvePush?.("ok");

		await waitFor(() => {
			expect(screen.queryByText("Pushing...")).not.toBeInTheDocument();
		});
	});

	it("should show success message after push completes", async () => {
		render(<SourceControlPanel rootPath="/test/repo" />);

		fireEvent.click(screen.getByText("Push"));

		await waitFor(() => {
			expect(screen.getByText("Pushed successfully")).toBeInTheDocument();
		});
	});

	describe("branch-base mode", () => {
		it("should show flat file list in branch-base mode", () => {
			mockEditorContext.diffBase = "branch-base";
			mockBranchDiffFiles.files = [
				{
					path: "src/app.tsx",
					old_path: null,
					status: "modified",
					binary: false,
					stats: { additions: 10, deletions: 3 },
				},
				{
					path: "src/new-file.ts",
					old_path: null,
					status: "added",
					binary: false,
					stats: { additions: 20, deletions: 0 },
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			expect(screen.getByText("2 file changes")).toBeInTheDocument();
			expect(screen.getByText(/Changed Files/)).toBeInTheDocument();
			expect(screen.getByText("app.tsx")).toBeInTheDocument();
			expect(screen.getByText("new-file.ts")).toBeInTheDocument();
			expect(screen.queryByText(/Unstaged Files/)).not.toBeInTheDocument();
			expect(screen.queryByText(/Staged Files/)).not.toBeInTheDocument();
		});

		it("should show error state when review diff fails", () => {
			mockEditorContext.diffBase = "branch-base";
			mockBranchDiffFiles.files = [];
			mockBranchDiffFiles.error = "failed to resolve base branch";
			render(<SourceControlPanel rootPath="/test/repo" />);

			expect(screen.getByText("Failed to load changes")).toBeInTheDocument();
			expect(
				screen.queryByText("No changes from base branch"),
			).not.toBeInTheDocument();
		});

		it("should show empty state when no changes from base branch", () => {
			mockEditorContext.diffBase = "branch-base";
			mockBranchDiffFiles.files = [];
			render(<SourceControlPanel rootPath="/test/repo" />);

			expect(
				screen.getByText("No changes from base branch"),
			).toBeInTheDocument();
		});

		it("should show staged mode when diffBase is staged", () => {
			mockEditorContext.diffBase = "staged";
			mockGitStatus.changedFiles = [
				{
					path: "file.txt",
					index_status: "none",
					worktree_status: "modified",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			expect(screen.getByText(/Unstaged Files/)).toBeInTheDocument();
			expect(
				screen.queryByText("No changes from base branch"),
			).not.toBeInTheDocument();
		});

		it("should call onSelectFile when clicking a branch-base file", () => {
			mockEditorContext.diffBase = "branch-base";
			mockBranchDiffFiles.files = [
				{
					path: "src/app.tsx",
					old_path: null,
					status: "modified",
					binary: false,
					stats: { additions: 5, deletions: 2 },
				},
			];
			const onSelectFile = vi.fn();
			render(
				<SourceControlPanel
					rootPath="/test/repo"
					onSelectFile={onSelectFile}
				/>,
			);

			fireEvent.click(screen.getByText("app.tsx"));

			expect(onSelectFile).toHaveBeenCalledWith("/test/repo/src/app.tsx");
		});

		it("should show file stats in branch-base mode", () => {
			mockEditorContext.diffBase = "branch-base";
			mockBranchDiffFiles.files = [
				{
					path: "file.txt",
					old_path: null,
					status: "modified",
					binary: false,
					stats: { additions: 15, deletions: 7 },
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			expect(screen.getByText("+15 -7")).toBeInTheDocument();
		});

		it("should render diffBase select with Staged and Branch Base options", async () => {
			const user = userEvent.setup();
			render(<SourceControlPanel rootPath="/test/repo" />);

			const trigger = screen.getByRole("combobox");
			expect(trigger).toBeInTheDocument();

			await user.click(trigger);

			await waitFor(() => {
				expect(
					screen.getByRole("option", { name: "Staged" }),
				).toBeInTheDocument();
				expect(
					screen.getByRole("option", { name: "Branch Base" }),
				).toBeInTheDocument();
			});

			await user.click(screen.getByRole("option", { name: "Branch Base" }));

			expect(mockEditorContext.setDiffBase).toHaveBeenCalledWith("branch-base");
		});

		it("should disable branch-base option when baseBranch is null", async () => {
			vi.mocked(useBaseBranch).mockReturnValue({
				baseBranch: null,
				setBaseBranch: vi.fn(),
				localBranches: [],
			});
			const user = userEvent.setup();
			render(<SourceControlPanel rootPath="/test/repo" />);

			const trigger = screen.getByRole("combobox");
			await user.click(trigger);

			await waitFor(() => {
				const branchBaseOption = screen.getByRole("option", {
					name: "Branch Base",
				});
				expect(branchBaseOption).toHaveAttribute("aria-disabled", "true");
			});
		});

		it("should disable branch-base option before initial commit (baseBranch=null)", async () => {
			vi.mocked(useBaseBranch).mockReturnValue({
				baseBranch: null,
				setBaseBranch: vi.fn(),
				localBranches: [],
			});
			mockGitStatus.changedFiles = [];
			mockGitStatus.stagedFiles = [];

			const user = userEvent.setup();
			render(<SourceControlPanel rootPath="/test/repo" />);

			const trigger = screen.getByRole("combobox");
			await user.click(trigger);

			await waitFor(() => {
				const branchBaseOption = screen.getByRole("option", {
					name: "Branch Base",
				});
				expect(branchBaseOption).toHaveAttribute("aria-disabled", "true");
			});

			const stagedOption = screen.getByRole("option", { name: "Staged" });
			expect(stagedOption).not.toHaveAttribute("aria-disabled");
		});
	});

	describe("context menu", () => {
		it("should show context menu on right-click for unstaged file", async () => {
			const user = userEvent.setup();
			mockGitStatus.changedFiles = [
				{
					path: "file.txt",
					index_status: "none",
					worktree_status: "modified",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			const fileItem = screen.getByText("file.txt");
			await user.pointer({ keys: "[MouseRight]", target: fileItem });

			await waitFor(() => {
				expect(screen.getByText("Open Changes")).toBeInTheDocument();
				expect(screen.getByText("Stage")).toBeInTheDocument();
				expect(screen.getByText("Discard")).toBeInTheDocument();
				expect(screen.getByText("Copy Path")).toBeInTheDocument();
				expect(screen.getByText("Copy Relative Path")).toBeInTheDocument();
				expect(screen.getByText("Reveal in Finder")).toBeInTheDocument();
			});
		});

		it("should show context menu on right-click for staged file", async () => {
			const user = userEvent.setup();
			mockGitStatus.stagedFiles = [
				{
					path: "staged.txt",
					index_status: "new",
					worktree_status: "none",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			const fileItem = screen.getByText("staged.txt");
			await user.pointer({ keys: "[MouseRight]", target: fileItem });

			await waitFor(() => {
				expect(screen.getByText("Open Changes")).toBeInTheDocument();
				expect(screen.getByText("Unstage")).toBeInTheDocument();
				expect(screen.queryByText("Discard")).not.toBeInTheDocument();
				expect(screen.getByText("Copy Path")).toBeInTheDocument();
			});
		});

		it("should show discard confirmation dialog", async () => {
			const user = userEvent.setup();
			mockGitStatus.changedFiles = [
				{
					path: "file.txt",
					index_status: "none",
					worktree_status: "modified",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			const fileItem = screen.getByText("file.txt");
			await user.pointer({ keys: "[MouseRight]", target: fileItem });

			await waitFor(() => {
				expect(screen.getByText("Discard")).toBeInTheDocument();
			});

			await user.click(screen.getByText("Discard"));

			await waitFor(() => {
				expect(screen.getByText("Discard Changes")).toBeInTheDocument();
				expect(
					screen.getByText(/Discard changes in "file\.txt"/),
				).toBeInTheDocument();
			});
		});

		it("should execute discard on confirmation", async () => {
			const user = userEvent.setup();
			mockGitStatus.changedFiles = [
				{
					path: "file.txt",
					index_status: "none",
					worktree_status: "modified",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			const fileItem = screen.getByText("file.txt");
			await user.pointer({ keys: "[MouseRight]", target: fileItem });

			await waitFor(() => {
				expect(screen.getByText("Discard")).toBeInTheDocument();
			});

			await user.click(screen.getByText("Discard"));

			await waitFor(() => {
				expect(screen.getByText("Discard Changes")).toBeInTheDocument();
			});

			await user.click(screen.getByText("Discard"));

			await waitFor(() => {
				expect(mockGitActions.discard).toHaveBeenCalledWith("/test/repo", [
					"file.txt",
				]);
			});
			expect(mockGitStatus.refresh).toHaveBeenCalled();
		});

		it("should stage via context menu", async () => {
			const user = userEvent.setup();
			mockGitStatus.changedFiles = [
				{
					path: "file.txt",
					index_status: "none",
					worktree_status: "modified",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			const fileItem = screen.getByText("file.txt");
			await user.pointer({ keys: "[MouseRight]", target: fileItem });

			await waitFor(() => {
				expect(screen.getByText("Stage")).toBeInTheDocument();
			});

			await user.click(screen.getByText("Stage"));

			await waitFor(() => {
				expect(mockGitActions.stage).toHaveBeenCalledWith("/test/repo", [
					"file.txt",
				]);
			});
		});

		it("should unstage via context menu", async () => {
			const user = userEvent.setup();
			mockGitStatus.stagedFiles = [
				{
					path: "file.txt",
					index_status: "new",
					worktree_status: "none",
				},
			];
			render(<SourceControlPanel rootPath="/test/repo" />);

			const fileItem = screen.getByText("file.txt");
			await user.pointer({ keys: "[MouseRight]", target: fileItem });

			await waitFor(() => {
				expect(screen.getByText("Unstage")).toBeInTheDocument();
			});

			await user.click(screen.getByText("Unstage"));

			await waitFor(() => {
				expect(mockGitActions.unstage).toHaveBeenCalledWith("/test/repo", [
					"file.txt",
				]);
			});
		});
	});
});
