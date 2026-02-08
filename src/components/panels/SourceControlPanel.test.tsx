import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
	commit: vi.fn().mockResolvedValue("abc123"),
	push: vi.fn().mockResolvedValue("ok"),
	createBranch: vi.fn().mockResolvedValue(undefined),
	switchBranch: vi.fn().mockResolvedValue(undefined),
};

vi.mock("@/hooks/useGitStatus", () => ({
	useGitStatus: () => mockGitStatus,
}));

vi.mock("@/hooks/useGitActions", () => ({
	useGitActions: () => mockGitActions,
}));

describe("SourceControlPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockGitStatus.stagedFiles = [];
		mockGitStatus.changedFiles = [];
		mockGitActions.stage.mockResolvedValue(undefined);
		mockGitActions.unstage.mockResolvedValue(undefined);
		mockGitActions.commit.mockResolvedValue("abc123");
		mockGitActions.push.mockResolvedValue("ok");
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
});
