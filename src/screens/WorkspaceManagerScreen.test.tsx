import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspaceManagerScreen } from "./WorkspaceManagerScreen";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockListen = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
	open: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/components/panels/RemotePanel", () => ({
	RemotePanel: () => <div data-testid="remote-panel">RemotePanel</div>,
}));

import type { BranchCard } from "@/types/git";

const todoBranch: BranchCard = {
	name: "feat/todo-branch",
	is_default: false,
	worktree_path: null,
	dirty_count: 0,
	is_merged: false,
};

const inProgressBranch: BranchCard = {
	name: "feat/active-branch",
	is_default: false,
	worktree_path: "/tmp/worktrees/active",
	dirty_count: 3,
	is_merged: false,
};

const inProgressCleanBranch: BranchCard = {
	name: "feat/clean-active",
	is_default: false,
	worktree_path: "/tmp/worktrees/clean-active",
	dirty_count: 0,
	is_merged: false,
};

const doneBranch: BranchCard = {
	name: "feat/merged-branch",
	is_default: false,
	worktree_path: null,
	dirty_count: 0,
	is_merged: true,
};

const allBranches: BranchCard[] = [
	todoBranch,
	inProgressBranch,
	inProgressCleanBranch,
	doneBranch,
];

function setupMockInvoke(branches: BranchCard[] = allBranches) {
	mockInvoke.mockImplementation((cmd: string) => {
		switch (cmd) {
			case "list_branches_with_status":
				return Promise.resolve(branches);
			case "get_releash_base":
				return Promise.resolve(null);
			case "get_default_branch":
				return Promise.resolve("main");
			default:
				return Promise.resolve(null);
		}
	});
}

function renderScreen(repoPath: string | null = "/home/user/my-repo") {
	const onSelectWorktree = vi.fn();
	const onChangeRepo = vi.fn();
	const result = render(
		<WorkspaceManagerScreen
			repoPath={repoPath}
			onSelectWorktree={onSelectWorktree}
			onChangeRepo={onChangeRepo}
		/>,
	);
	return { ...result, onSelectWorktree, onChangeRepo };
}

beforeEach(() => {
	vi.clearAllMocks();
	mockListen.mockResolvedValue(() => {});
	setupMockInvoke();
});

afterEach(() => {
	vi.restoreAllMocks();
});

describe("WorkspaceManagerScreen", () => {
	describe("画面切替", () => {
		it("repoPath=null で Open Folder ボタンを表示", () => {
			renderScreen(null);
			expect(screen.getByText("Open Folder")).toBeInTheDocument();
			expect(screen.queryByText("Todo")).not.toBeInTheDocument();
		});

		it("repoPath ありで Kanban ボードを表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});
			expect(screen.getByText("In Progress")).toBeInTheDocument();
			expect(screen.getByText("Done")).toBeInTheDocument();
		});

		it("リポジトリ名をヘッダーに表示", async () => {
			renderScreen("/home/user/my-repo");
			await waitFor(() => {
				expect(screen.getByText("my-repo")).toBeInTheDocument();
			});
		});

		it("ベースブランチラベルを表示（auto検出時）", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("base: main (auto)")).toBeInTheDocument();
			});
		});

		it("ベースブランチラベルを表示（手動設定時）", async () => {
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve(allBranches);
					case "get_releash_base":
						return Promise.resolve("develop");
					default:
						return Promise.resolve(null);
				}
			});
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("base: develop")).toBeInTheDocument();
			});
		});
	});

	describe("Kanban 分類ロジック", () => {
		it("is_merged=true のブランチが Done 列に表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/merged-branch")).toBeInTheDocument();
			});
			expect(screen.getByText("merged")).toBeInTheDocument();
		});

		it("worktree_path ありのブランチが In Progress 列に表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/active-branch")).toBeInTheDocument();
			});
			expect(screen.getByText("3 files changed")).toBeInTheDocument();
		});

		it("worktree_path なし & is_merged=false のブランチが Todo 列に表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/todo-branch")).toBeInTheDocument();
			});
		});

		it("各列のカウント表示が正しい", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});
			// Todo: 1, In Progress: 2, Done: 1
			const counts = screen.getAllByText(/^[0-9]+$/);
			const countValues = counts.map((el) => el.textContent);
			expect(countValues).toContain("1"); // Todo or Done
			expect(countValues).toContain("2"); // In Progress
		});

		it("dirty_count=0 で worktree がある場合 clean と表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/clean-active")).toBeInTheDocument();
			});
			expect(screen.getByText("clean")).toBeInTheDocument();
		});
	});

	describe("Worktree 操作", () => {
		it("worktree ありのブランチの Open で onSelectWorktree が呼ばれる", async () => {
			const user = userEvent.setup();
			const { onSelectWorktree } = renderScreen();

			await waitFor(() => {
				expect(screen.getByText("feat/active-branch")).toBeInTheDocument();
			});

			screen.getAllByText("Open");
			// active-branch の Open ボタンをクリック（2番目: In Progress列の1つ目）
			const activeBranchCard = screen
				.getByText("feat/active-branch")
				.closest("[class*='flex flex-col gap-3']");
			const openBtn = activeBranchCard?.querySelector("button");
			if (openBtn) {
				await user.click(openBtn);
			}

			await waitFor(() => {
				expect(onSelectWorktree).toHaveBeenCalledWith("/tmp/worktrees/active");
			});
		});

		it("worktree なしのブランチの Open で create_worktree が呼ばれる", async () => {
			const user = userEvent.setup();
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve(allBranches);
					case "get_releash_base":
						return Promise.resolve(null);
					case "get_default_branch":
						return Promise.resolve("main");
					case "create_worktree":
						return Promise.resolve({
							name: "feat-todo-branch",
							path: "/tmp/worktrees/todo",
							branch: "feat/todo-branch",
							is_main: false,
							is_locked: false,
							dirty_count: 0,
							base_branch: null,
						});
					default:
						return Promise.resolve(null);
				}
			});

			const { onSelectWorktree } = renderScreen();

			await waitFor(() => {
				expect(screen.getByText("feat/todo-branch")).toBeInTheDocument();
			});

			const todoCard = screen
				.getByText("feat/todo-branch")
				.closest("[class*='flex flex-col gap-3']");
			const openBtn = todoCard?.querySelector("button");
			if (openBtn) {
				await user.click(openBtn);
			}

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("create_worktree", {
					repoPath: "/home/user/my-repo",
					worktreePath: "/home/user/my-repo-worktrees/feat-todo-branch",
					branch: "feat/todo-branch",
					createBranch: false,
					baseBranch: null,
				});
			});
			expect(onSelectWorktree).toHaveBeenCalledWith("/tmp/worktrees/todo");
		});

		it("worktree ありのブランチに削除ボタンが表示される", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/active-branch")).toBeInTheDocument();
			});
			expect(
				screen.getByLabelText("Delete worktree for feat/active-branch"),
			).toBeInTheDocument();
		});

		it("worktree なしのブランチに削除ボタンが表示されない", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/todo-branch")).toBeInTheDocument();
			});
			expect(
				screen.queryByLabelText("Delete worktree for feat/todo-branch"),
			).not.toBeInTheDocument();
		});
	});

	describe("リモートパネル表示切替", () => {
		it("Remote ボタンでパネルの表示/非表示が切り替わる", async () => {
			const user = userEvent.setup();
			renderScreen();

			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});

			expect(screen.queryByTestId("remote-panel")).not.toBeInTheDocument();

			const remoteBtn = screen.getByTitle("Remote");
			await user.click(remoteBtn);

			expect(screen.getByTestId("remote-panel")).toBeInTheDocument();

			await user.click(remoteBtn);

			expect(screen.queryByTestId("remote-panel")).not.toBeInTheDocument();
		});
	});

	describe("エラー状態", () => {
		it("list_branches_with_status が失敗した場合にエラーをコンソールに出力", async () => {
			const consoleSpy = vi
				.spyOn(console, "error")
				.mockImplementation(() => {});
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.reject(new Error("network error"));
					case "get_releash_base":
						return Promise.resolve(null);
					case "get_default_branch":
						return Promise.resolve("main");
					default:
						return Promise.resolve(null);
				}
			});

			renderScreen();

			await waitFor(() => {
				expect(consoleSpy).toHaveBeenCalledWith(
					"Failed to list branches:",
					expect.any(Error),
				);
			});

			// loading=false になり Kanban 列は表示されるが空
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});

			consoleSpy.mockRestore();
		});

		it("get_releash_base が失敗した場合にベースブランチラベルが空", async () => {
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve([]);
					case "get_releash_base":
						return Promise.reject(new Error("config error"));
					default:
						return Promise.resolve(null);
				}
			});

			renderScreen();

			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});

			expect(screen.queryByText(/base:/)).not.toBeInTheDocument();
		});
	});

	describe("イベント駆動更新", () => {
		it("branch-list-sync イベントで listen が登録される", async () => {
			renderScreen();

			await waitFor(() => {
				expect(mockListen).toHaveBeenCalledWith(
					"branch-list-sync",
					expect.any(Function),
				);
			});
		});

		it("branch-list-sync イベント受信時にブランチリストが更新される", async () => {
			let listenCallback: (() => void) | null = null;
			mockListen.mockImplementation((_event: string, cb: () => void) => {
				listenCallback = cb;
				return Promise.resolve(() => {});
			});

			renderScreen();

			await waitFor(() => {
				expect(screen.getByText("feat/todo-branch")).toBeInTheDocument();
			});

			// invoke をリセットして新しいデータを返す
			const updatedBranches: BranchCard[] = [
				{
					name: "feat/new-branch",
					is_default: false,
					worktree_path: null,
					dirty_count: 0,
					is_merged: false,
				},
			];
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve(updatedBranches);
					case "get_releash_base":
						return Promise.resolve(null);
					case "get_default_branch":
						return Promise.resolve("main");
					default:
						return Promise.resolve(null);
				}
			});

			// イベントコールバックを発火
			await act(async () => {
				listenCallback?.();
			});

			await waitFor(() => {
				expect(screen.getByText("feat/new-branch")).toBeInTheDocument();
			});
			expect(screen.queryByText("feat/todo-branch")).not.toBeInTheDocument();
		});

		it("repoPath=null の場合 listen が呼ばれない", () => {
			renderScreen(null);
			expect(mockListen).not.toHaveBeenCalled();
		});

		it("フォールバックポーリングが 30秒間隔で設定される", async () => {
			vi.useFakeTimers();
			setupMockInvoke();
			renderScreen();

			await act(async () => {
				await vi.advanceTimersByTimeAsync(100);
			});

			const callCount = mockInvoke.mock.calls.filter(
				(c) => c[0] === "list_branches_with_status",
			).length;

			// 30秒未満では追加の呼び出しなし
			await act(async () => {
				await vi.advanceTimersByTimeAsync(29_000);
			});

			const callCountAfter29s = mockInvoke.mock.calls.filter(
				(c) => c[0] === "list_branches_with_status",
			).length;
			expect(callCountAfter29s).toBe(callCount);

			// 30秒経過で呼び出しが発生
			Object.defineProperty(document, "visibilityState", {
				value: "visible",
				writable: true,
			});
			await act(async () => {
				await vi.advanceTimersByTimeAsync(1_000);
			});

			const callCountAfter30s = mockInvoke.mock.calls.filter(
				(c) => c[0] === "list_branches_with_status",
			).length;
			expect(callCountAfter30s).toBeGreaterThan(callCount);

			vi.useRealTimers();
		});
	});
});
