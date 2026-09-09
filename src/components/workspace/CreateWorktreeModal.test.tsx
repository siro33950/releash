import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { IssueInfo, WorktreeBranch, WorktreeEntry } from "@/types/git";
import type { NotionTask } from "@/types/notion";
import { CreateWorktreeModal } from "./CreateWorktreeModal";

const hookMocks = vi.hoisted(() => ({
	useIssues: vi.fn(),
	useNotionLabelOptions: vi.fn(),
	useNotionTasks: vi.fn(),
}));

vi.mock("@/hooks/useIssues", () => ({
	useIssues: hookMocks.useIssues,
}));

vi.mock("@/hooks/useNotionLabelOptions", () => ({
	useNotionLabelOptions: hookMocks.useNotionLabelOptions,
}));

vi.mock("@/hooks/useNotionTasks", () => ({
	useNotionTasks: hookMocks.useNotionTasks,
}));

const mockInvoke = vi.mocked(invoke);

function makeIssue(overrides: Partial<IssueInfo> = {}): IssueInfo {
	return {
		number: 1302,
		default_branch_name: "backend/issue-1302",
		title: "Move branch rules to Rust",
		state: "OPEN",
		url: "https://github.com/releash/releash/issues/1302",
		author: { login: "siro" },
		created_at: "2026-01-01T00:00:00Z",
		updated_at: "2026-01-02T00:00:00Z",
		labels: [],
		assignees: [],
		body: "",
		milestone: null,
		...overrides,
	};
}

function makeBranch(overrides: Partial<WorktreeBranch> = {}): WorktreeBranch {
	return {
		name: "main",
		is_main_worktree: true,
		worktree_path: "/repo",
		dirty_count: 0,
		is_merged: false,
		ahead: 0,
		behind: 0,
		has_upstream: false,
		base_ahead: 0,
		...overrides,
	};
}

function makeNotionTask(overrides: Partial<NotionTask> = {}): NotionTask {
	return {
		id: "notion-page-1",
		title: "Move Notion branch rules",
		url: "https://notion.so/page-1",
		labels: {},
		branch_name: "notion/page1",
		created_at: "2026-01-01T00:00:00Z",
		last_edited_at: "2026-01-02T00:00:00Z",
		...overrides,
	};
}

describe("CreateWorktreeModal", () => {
	let branchCards: WorktreeBranch[];

	beforeEach(() => {
		vi.clearAllMocks();
		branchCards = [];
		hookMocks.useIssues.mockReturnValue({
			issues: [],
			loading: false,
			refresh: vi.fn(),
		});
		hookMocks.useNotionLabelOptions.mockReturnValue({
			labelOptions: [],
			loading: false,
		});
		hookMocks.useNotionTasks.mockReturnValue({
			tasks: [],
			loading: false,
			loadMore: vi.fn(),
			hasMore: false,
			search: vi.fn(),
			refresh: vi.fn(),
		});
		mockInvoke.mockImplementation((command: string, args?: unknown) => {
			if (command === "list_branches") {
				return Promise.resolve([{ name: "main", is_remote: false }]);
			}
			if (command === "list_branches_with_status_snapshot") {
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					limited: false,
					branches: branchCards,
				});
			}
			if (command === "create_worktree") {
				const branch = (args as { branch: string }).branch;
				return Promise.resolve({
					name: "created-worktree",
					path: "/fixture/worktree",
					branch,
					is_main: false,
					is_locked: false,
					dirty_count: 0,
					base_branch: "main",
				} satisfies WorktreeEntry);
			}
			return Promise.resolve([]);
		});
	});

	it("create_worktree に frontend 導出の worktreePath を渡さない", async () => {
		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);

		fireEvent.change(screen.getByLabelText("Branch name"), {
			target: { value: "feat/plain" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Create" }));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"create_worktree",
				expect.objectContaining({
					repoPath: "/repo",
					branch: "feat/plain",
					createBranch: true,
				}),
			);
		});
		const createArgs = mockInvoke.mock.calls.find(
			([command]) => command === "create_worktree",
		)?.[1] as Record<string, unknown>;
		expect(createArgs).not.toHaveProperty("worktreePath");
	});

	it("issue の default_branch_name を作成対象 branch として使う", async () => {
		const user = userEvent.setup();
		hookMocks.useIssues.mockReturnValue({
			issues: [makeIssue()],
			loading: false,
			refresh: vi.fn(),
		});

		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);

		await user.click(screen.getByRole("tab", { name: /Issue/ }));
		await user.click(
			await screen.findByRole("button", { name: /Move branch rules to Rust/ }),
		);
		await user.click(screen.getByRole("button", { name: "Create" }));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"create_worktree",
				expect.objectContaining({
					branch: "backend/issue-1302",
				}),
			);
		});
	});

	it("既存 worktree の branch と一致する issue を候補から除外する", async () => {
		const user = userEvent.setup();
		branchCards = [
			makeBranch({
				name: "backend/issue-1302",
				is_main_worktree: false,
				worktree_path: "/repo-worktrees/backend-issue-1302",
			}),
		];
		hookMocks.useIssues.mockReturnValue({
			issues: [makeIssue()],
			loading: false,
			refresh: vi.fn(),
		});

		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);

		await user.click(screen.getByRole("tab", { name: /Issue/ }));

		await waitFor(() => {
			expect(screen.getByText("No issues found")).toBeInTheDocument();
		});
	});

	it("notion task の backend-provided branch_name を作成対象 branch として使う", async () => {
		const user = userEvent.setup();
		hookMocks.useNotionTasks.mockReturnValue({
			tasks: [makeNotionTask()],
			loading: false,
			loadMore: vi.fn(),
			hasMore: false,
			search: vi.fn(),
			refresh: vi.fn(),
		});

		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);

		await user.click(screen.getByRole("tab", { name: /Notion/ }));
		await user.click(
			await screen.findByRole("button", {
				name: /Move Notion branch rules/,
			}),
		);
		await user.click(screen.getByRole("button", { name: "Create" }));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"create_worktree",
				expect.objectContaining({
					branch: "notion/page1",
				}),
			);
		});
	});

	it("branch property 未設定の notion task は backend fallback branch で候補に残る", async () => {
		const user = userEvent.setup();
		hookMocks.useNotionTasks.mockReturnValue({
			tasks: [makeNotionTask({ branch_name: "feat/move-notion-branch-rules" })],
			loading: false,
			loadMore: vi.fn(),
			hasMore: false,
			search: vi.fn(),
			refresh: vi.fn(),
		});

		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);

		await user.click(screen.getByRole("tab", { name: /Notion/ }));
		await user.click(
			await screen.findByRole("button", {
				name: /Move Notion branch rules/,
			}),
		);
		await user.click(screen.getByRole("button", { name: "Create" }));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"create_worktree",
				expect.objectContaining({
					branch: "feat/move-notion-branch-rules",
				}),
			);
		});
	});
});
