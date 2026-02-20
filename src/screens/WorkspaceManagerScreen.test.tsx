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

vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: vi.fn(),
}));

vi.mock("react-resizable-panels", () => {
	const Panel = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel">{children}</div>
	);
	const Group = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel-group">{children}</div>
	);
	const Separator = () => <div data-testid="separator" />;
	return { Panel, Group, Separator };
});

vi.mock("@/components/panels/RemotePanel", () => ({
	RemotePanel: () => <div data-testid="remote-panel">RemotePanel</div>,
}));

import type { BranchCard, ProviderStatus } from "@/types/git";
import { DEFAULT_SETTINGS } from "@/types/settings";

const todoBranch: BranchCard = {
	name: "feat/todo-branch",
	is_default: false,
	worktree_path: null,
	dirty_count: 0,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

const inProgressBranch: BranchCard = {
	name: "feat/active-branch",
	is_default: false,
	worktree_path: "/tmp/worktrees/active",
	dirty_count: 3,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

const inProgressCleanBranch: BranchCard = {
	name: "feat/clean-active",
	is_default: false,
	worktree_path: "/tmp/worktrees/clean-active",
	dirty_count: 0,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

const reviewBranch: BranchCard = {
	name: "feat/review-branch",
	is_default: false,
	worktree_path: "/tmp/worktrees/review",
	dirty_count: 1,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

const doneBranch: BranchCard = {
	name: "feat/merged-branch",
	is_default: false,
	worktree_path: null,
	dirty_count: 0,
	is_merged: true,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

const allBranches: BranchCard[] = [
	todoBranch,
	inProgressBranch,
	inProgressCleanBranch,
	reviewBranch,
	doneBranch,
];

import type { PrStatus } from "@/types/git";

const defaultPrStatus: PrStatus = {
	open_prs: {
		"feat/review-branch": {
			number: 42,
			url: "https://github.com/owner/repo/pull/42",
		},
	},
	merged_branches: [],
};

function setupMockInvoke(
	branches: BranchCard[] = allBranches,
	prStatus: PrStatus = defaultPrStatus,
) {
	mockInvoke.mockImplementation((cmd: string) => {
		switch (cmd) {
			case "list_branches_with_status":
				return Promise.resolve(branches);
			case "get_cached_pr_status":
				return Promise.resolve(prStatus);
			case "get_cached_issues":
				return Promise.resolve([]);
			case "get_releash_base":
				return Promise.resolve(null);
			case "get_default_branch":
				return Promise.resolve("main");
			case "get_agent_states":
				return Promise.resolve({});
			case "get_notify_config":
				return Promise.resolve({
					webhook_url: "",
					on_running: false,
					on_done: true,
					on_error: true,
					on_waiting: true,
					desktop_mode: "always",
					inactive_timeout_minutes: 2,
				});
			case "get_remote_config":
				return Promise.resolve({
					auto_start: false,
					auto_start_on_lan: false,
				});
			default:
				return Promise.resolve(null);
		}
	});
}

const REPO_PATH = "/home/user/my-repo";

function renderScreen(repoPaths: string[] = [REPO_PATH]) {
	const onSelectWorktree = vi.fn();
	const onAddRepo = vi.fn();
	const onRemoveRepo = vi.fn();
	const providerStatuses: Record<string, ProviderStatus | null> = {};
	for (const p of repoPaths) {
		providerStatuses[p] = "available";
	}
	const result = render(
		<WorkspaceManagerScreen
			repoPaths={repoPaths}
			settings={DEFAULT_SETTINGS}
			providerStatuses={providerStatuses}
			onSettingsSave={vi.fn()}
			onSelectWorktree={onSelectWorktree}
			onAddRepo={onAddRepo}
			onRemoveRepo={onRemoveRepo}
		/>,
	);
	return { ...result, onSelectWorktree, onAddRepo, onRemoveRepo };
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
		it("repoPaths=[] で Open Folder ボタンを表示", () => {
			renderScreen([]);
			expect(screen.getByText("Open Folder")).toBeInTheDocument();
			expect(screen.queryByText("Todo")).not.toBeInTheDocument();
		});

		it("repoPaths ありで Kanban ボードを表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});
			expect(screen.getByText("In Progress")).toBeInTheDocument();
			expect(screen.getByText("Review")).toBeInTheDocument();
			expect(screen.getByText("Done")).toBeInTheDocument();
		});

		it("リポジトリ名をヘッダーに表示", async () => {
			renderScreen(["/home/user/my-repo"]);
			await waitFor(() => {
				const elements = screen.getAllByText("my-repo");
				expect(elements.length).toBeGreaterThanOrEqual(1);
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
					case "get_cached_pr_status":
						return Promise.resolve(defaultPrStatus);
					case "get_cached_issues":
						return Promise.resolve([]);
					case "get_releash_base":
						return Promise.resolve("develop");
					case "get_agent_states":
						return Promise.resolve({});
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
			expect(screen.getByText("3 changed")).toBeInTheDocument();
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
			// Todo: 1, In Progress: 2, Review: 1, Done: 1
			const counts = screen.getAllByText(/^[0-9]+$/);
			const countValues = counts.map((el) => el.textContent);
			expect(countValues).toContain("1"); // Todo, Review, or Done
			expect(countValues).toContain("2"); // In Progress
		});

		it("PR enrichment 後にブランチが Review 列に表示", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("#42")).toBeInTheDocument();
			});
			expect(screen.getByText("Review")).toBeInTheDocument();
		});

		it("PRバッジが表示される", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("#42")).toBeInTheDocument();
			});
			const prBadge = screen.getByText("#42");
			expect(prBadge.closest("button")).toBeInTheDocument();
		});

		it("is_merged が has_pr より優先される", async () => {
			const mergedWithPr: BranchCard = {
				name: "feat/merged-with-pr",
				is_default: false,
				worktree_path: null,
				dirty_count: 0,
				is_merged: true,
				has_pr: false,
				pr_number: null,
				pr_url: null,
				ahead: 0,
				behind: 0,
				is_remote_only: false,
				has_upstream: true,
				remote_name: null,
			};
			const prStatus: PrStatus = {
				open_prs: {
					"feat/merged-with-pr": {
						number: 99,
						url: "https://github.com/owner/repo/pull/99",
					},
				},
				merged_branches: [],
			};
			setupMockInvoke([mergedWithPr], prStatus);
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/merged-with-pr")).toBeInTheDocument();
			});
			expect(screen.getByText("merged")).toBeInTheDocument();
		});

		it("dirty_count=0 で worktree がある場合 clean バッジを表示しない", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("feat/clean-active")).toBeInTheDocument();
			});
			expect(screen.queryByText("clean")).not.toBeInTheDocument();
		});
	});

	describe("Worktree 操作", () => {
		it("worktree ありのブランチの Open で onSelectWorktree が呼ばれる", async () => {
			const user = userEvent.setup();
			const { onSelectWorktree } = renderScreen();

			await waitFor(() => {
				expect(screen.getByText("feat/active-branch")).toBeInTheDocument();
			});

			const activeBranchCard = screen.getByTestId(
				"branch-card-feat/active-branch",
			);
			await user.click(activeBranchCard);

			await waitFor(() => {
				expect(onSelectWorktree).toHaveBeenCalledWith(
					"/tmp/worktrees/active",
					"feat/active-branch",
					"my-repo",
				);
			});
		});

		it("worktree なしのブランチの Open で create_worktree が呼ばれる", async () => {
			const user = userEvent.setup();
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve(allBranches);
					case "get_cached_pr_status":
						return Promise.resolve(defaultPrStatus);
					case "get_cached_issues":
						return Promise.resolve([]);
					case "get_releash_base":
						return Promise.resolve(null);
					case "get_default_branch":
						return Promise.resolve("main");
					case "get_agent_states":
						return Promise.resolve({});
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

			const todoCard = screen.getByTestId("branch-card-feat/todo-branch");
			await user.click(todoCard);

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("create_worktree", {
					repoPath: "/home/user/my-repo",
					worktreePath: "/home/user/my-repo-worktrees/feat-todo-branch",
					branch: "feat/todo-branch",
					createBranch: false,
					baseBranch: null,
				});
			});
			expect(onSelectWorktree).toHaveBeenCalledWith(
				"/tmp/worktrees/todo",
				"feat/todo-branch",
				"my-repo",
			);
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

	describe("サイドバーパネル表示切替", () => {
		it("初期状態で Issues パネルが表示される", async () => {
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});
			expect(screen.getByText("Issues")).toBeInTheDocument();
		});

		it("ActivityBar の Remote ボタンで Remote パネルに切り替わる", async () => {
			const user = userEvent.setup();
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});

			const remoteBtn = screen.getByLabelText("Remote");
			await user.click(remoteBtn);

			expect(screen.getByTestId("remote-panel")).toBeInTheDocument();
		});

		it("ActivityBar の Settings ボタンで設定モーダルが開く", async () => {
			const user = userEvent.setup();
			renderScreen();
			await waitFor(() => {
				expect(screen.getByText("Todo")).toBeInTheDocument();
			});

			const settingsBtn = screen.getByLabelText("Settings");
			await user.click(settingsBtn);

			expect(screen.getByRole("dialog")).toBeInTheDocument();
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
					case "get_cached_issues":
						return Promise.resolve([]);
					case "get_releash_base":
						return Promise.resolve(null);
					case "get_default_branch":
						return Promise.resolve("main");
					case "get_agent_states":
						return Promise.resolve({});
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

			await waitFor(() => {
				expect(
					screen.getByText("ワークツリーがありません"),
				).toBeInTheDocument();
			});

			consoleSpy.mockRestore();
		});

		it("get_releash_base が失敗した場合にベースブランチラベルが空", async () => {
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve([]);
					case "get_cached_pr_status":
						return Promise.resolve({ open_prs: {}, merged_branches: [] });
					case "get_cached_issues":
						return Promise.resolve([]);
					case "get_releash_base":
						return Promise.reject(new Error("config error"));
					case "get_agent_states":
						return Promise.resolve({});
					default:
						return Promise.resolve(null);
				}
			});

			renderScreen();

			await waitFor(() => {
				expect(
					screen.getByText("ワークツリーがありません"),
				).toBeInTheDocument();
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
			mockListen.mockImplementation((event: string, cb: () => void) => {
				if (event === "branch-list-sync") {
					listenCallback = cb;
				}
				return Promise.resolve(() => {});
			});

			renderScreen();

			await waitFor(() => {
				expect(screen.getByText("feat/todo-branch")).toBeInTheDocument();
			});

			const updatedBranches: BranchCard[] = [
				{
					name: "feat/new-branch",
					is_default: false,
					worktree_path: null,
					dirty_count: 0,
					is_merged: false,
					has_pr: false,
					pr_number: null,
					pr_url: null,
					ahead: 0,
					behind: 0,
					is_remote_only: false,
					has_upstream: true,
					remote_name: null,
				},
			];
			mockInvoke.mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches_with_status":
						return Promise.resolve(updatedBranches);
					case "get_cached_pr_status":
						return Promise.resolve({
							open_prs: {},
							merged_branches: [],
						});
					case "get_cached_issues":
						return Promise.resolve([]);
					case "get_releash_base":
						return Promise.resolve(null);
					case "get_default_branch":
						return Promise.resolve("main");
					case "get_agent_states":
						return Promise.resolve({});
					default:
						return Promise.resolve(null);
				}
			});

			await act(async () => {
				listenCallback?.();
			});

			await waitFor(() => {
				expect(screen.getByText("feat/new-branch")).toBeInTheDocument();
			});
			expect(screen.queryByText("feat/todo-branch")).not.toBeInTheDocument();
		});

		it("repoPaths=[] の場合 branch-list-sync の listen が呼ばれない", () => {
			renderScreen([]);
			expect(mockListen).not.toHaveBeenCalledWith(
				"branch-list-sync",
				expect.any(Function),
			);
		});
	});
});
