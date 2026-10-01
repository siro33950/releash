import { Code, ConnectError } from "@connectrpc/connect";
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BranchStatus } from "@/generated/client_types";
import { invokeClient as invoke, subscribeState } from "@/lib/client";
import type { IssueInfo, WorktreeEntry } from "@/types/git";
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
	let branchStatuses: BranchStatus[];

	beforeEach(() => {
		vi.clearAllMocks();
		branchStatuses = [];
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
		vi.mocked(subscribeState).mockImplementation((target, receive) => {
			const kind = typeof target === "string" ? target : target.kind;
			if (kind === "branches") receive([{ name: "main", is_remote: false }]);
			if (kind === "branch-status") receive(branchStatuses);
			return vi.fn();
		});
		mockInvoke.mockImplementation((command: string, args?: unknown) => {
			if (command === "create_worktree") {
				const branch = (args as { branch: string }).branch;
				return Promise.resolve({
					name: "created-worktree",
					path: "/fixture/worktree",
					branch,
					is_main: false,
					is_locked: false,
				} satisfies WorktreeEntry);
			}
			return Promise.resolve([]);
		});
	});

	it("ブランチ読取の失敗を表示し成功後に解除する", () => {
		let recover!: Parameters<typeof subscribeState>[1];
		vi.mocked(subscribeState).mockImplementation((target, receive, fail) => {
			if (typeof target !== "string" && target.kind === "branches") {
				recover = receive;
				fail(new Error("branches unavailable"));
			} else receive([]);
			return vi.fn();
		});
		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);
		expect(screen.getByRole("alert")).toHaveTextContent("branches unavailable");
		act(() => recover([{ name: "main", is_remote: false }]));
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});

	it("同じRepository一覧の新しい配列でも入力と取得済み候補を保持する", async () => {
		const user = userEvent.setup();
		const props = { open: true, onCreated: vi.fn(), onClose: vi.fn() };
		const { rerender } = render(
			<CreateWorktreeModal {...props} repoPaths={["/repo"]} />,
		);
		await waitFor(() =>
			expect(subscribeState).toHaveBeenCalledWith(
				{ kind: "branch-status", args: ["/repo"] },
				expect.any(Function),
				expect.any(Function),
			),
		);
		await user.type(
			screen.getByPlaceholderText("feat/my-feature"),
			"feat/retained",
		);
		const reads = mockInvoke.mock.calls.length;
		rerender(<CreateWorktreeModal {...props} repoPaths={["/repo"]} />);
		expect(screen.getByPlaceholderText("feat/my-feature")).toHaveValue(
			"feat/retained",
		);
		expect(mockInvoke.mock.calls).toHaveLength(reads);
	});

	it.each([
		[
			new ConnectError("Request failed", Code.Unavailable),
			"Failed to create: feat/new",
		],
		[
			new ConnectError("Request failed", Code.Unavailable),
			"Failed to create: feat/new",
		],
		[new Error("backend failure"), "Failed to create: feat/new"],
	])("作成エラーに通信状態を表示しない: %s", async (failure, expected) => {
		const base = mockInvoke.getMockImplementation();
		if (!base) throw new Error("Missing worktree fixture");
		mockInvoke.mockImplementation((command, args) =>
			command === "create_worktree"
				? Promise.reject(failure)
				: base(command, args),
		);
		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={vi.fn()}
				onClose={vi.fn()}
			/>,
		);
		fireEvent.change(screen.getByLabelText("Branch name"), {
			target: { value: "feat/new" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Create" }));
		await waitFor(() =>
			expect(screen.getByText(new RegExp(expected))).toBeInTheDocument(),
		);
		expect(
			screen.queryByText(/未実行|未送信|結果不明|操作結果を確認できません/),
		).not.toBeInTheDocument();
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
		branchStatuses = [{ name: "backend/issue-1302", has_worktree: true }];
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

	it("通信状態を表示せず作成成功を後続処理へ渡す", async () => {
		const base = mockInvoke.getMockImplementation();
		if (!base) throw new Error("Missing invoke fixture");
		let complete!: (entry: WorktreeEntry) => void;
		mockInvoke.mockImplementation((command, args) => {
			if (command !== "create_worktree") return base(command, args);
			return new Promise((resolve) => {
				complete = resolve;
			});
		});
		const onCreated = vi.fn();
		render(
			<CreateWorktreeModal
				open
				repoPaths={["/repo"]}
				onCreated={onCreated}
				onClose={vi.fn()}
			/>,
		);
		fireEvent.change(screen.getByLabelText("Branch name"), {
			target: { value: "new" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Create" }));
		expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
		expect(screen.getByRole("button", { name: "Creating..." })).toBeDisabled();
		expect(
			screen.queryByText(/操作結果を確認できません/),
		).not.toBeInTheDocument();
		await act(async () =>
			complete({
				name: "new",
				path: "/repo/new",
				branch: "new",
				is_main: false,
				is_locked: false,
			}),
		);
		expect(onCreated).toHaveBeenCalledExactlyOnceWith(
			"/repo/new",
			"new",
			"repo",
		);
		expect(
			screen.queryByText(/操作結果を確認できません/),
		).not.toBeInTheDocument();
	});
});
