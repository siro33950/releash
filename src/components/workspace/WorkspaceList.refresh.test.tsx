import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	useWorkspaceList,
	type WorkspaceListModel,
} from "@/hooks/useWorkspaceList";
import { workspaceListSnapshot } from "@/test/workspaceList";
import type { WorkspaceTreeSnapshot } from "@/types/workspace-tree";
import { WorkspaceList as WorkspaceListView } from "./WorkspaceList";

let currentModel: WorkspaceListModel;

function WorkspaceList(
	props: Omit<React.ComponentProps<typeof WorkspaceListView>, "model">,
) {
	const model = useWorkspaceList();
	currentModel = model;
	return <WorkspaceListView {...props} model={model} />;
}

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@/lib/client", () => ({
	invokeClient: mocks.invoke,
	listenClient: mocks.listen,
	watchClient: () => () => {},
}));
vi.mock("@/hooks/useWorkflowConfig", () => ({
	useWorkflowConfig: () => ({ workflows: [], loading: false, error: null }),
}));

const tree: WorkspaceTreeSnapshot = {
	nodes: [
		{
			kind: "node",
			processPresence: "live",
			id: "session",
			title: "Running session",
			status: "active",
			contentKind: "session",
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: false,
				canResumeSession: false,
			},
			pastAttempts: [],
			pastAttemptsCollapsed: false,
			updatedAt: 1,
		},
	],
	archivedSessions: [],
	preferredNodeId: "session",
};
function setup() {
	const onSelectWorktree = vi.fn();
	const onWorkspaceSelectionInvalidated = vi.fn();
	const view = render(
		<WorkspaceList
			selectedRootPath="/repo"
			centerSelection={{
				kind: "node",
				worktreePath: "/repo",
				nodeId: "session",
			}}
			onSelectWorktree={onSelectWorktree}
			onWorkspaceSelectionInvalidated={onWorkspaceSelectionInvalidated}
			onAddRepo={vi.fn()}
			onShowSettings={vi.fn()}
		/>,
	);
	return { ...view, onSelectWorktree, onWorkspaceSelectionInvalidated };
}
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe("Workspaces refresh", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mocks.invoke.mockResolvedValue(workspaceListSnapshot(tree));
		mocks.listen.mockResolvedValue(vi.fn());
	});

	it("RepositoryのRustの取得失敗はその行だけに表示し全体エラーは局所成功後も残す", async () => {
		const initial = workspaceListSnapshot(tree);
		initial.repositories.push({
			...workspaceListSnapshot().repositories[0],
			path: "/other",
			branches: [],
			worktrees: [],
		});
		mocks.invoke.mockResolvedValue(initial);
		const { container } = setup();
		const session = await screen.findByText("Running session");
		const repoSection = screen.getByRole("button", { name: "repo1" })
			.parentElement?.parentElement;
		const otherSection = screen.getByRole("button", { name: "other0" })
			.parentElement?.parentElement;
		const repositoryFailed = structuredClone(initial);
		repositoryFailed.repositories[0].status = {
			loaded: true,
			state: "refreshFailed",
			error: "repository offline",
		};
		mocks.invoke.mockResolvedValueOnce(repositoryFailed);
		await act(async () => {
			await currentModel.refreshRepository("/repo");
		});
		const alert = screen.getByRole("alert");
		expect(alert).toHaveTextContent(
			"Refresh failed. Showing previous information",
		);
		expect(alert).toHaveTextContent("repository offline");
		expect(repoSection).toContainElement(alert);
		expect(otherSection).not.toContainElement(alert);
		expect(screen.getByText("Running session")).toBe(session);
		await act(async () => {
			await currentModel.refreshRepository("/repo");
		});
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		const failed = structuredClone(initial);
		failed.status = {
			loaded: true,
			state: "refreshFailed",
			error: "all offline",
		};
		mocks.invoke.mockResolvedValue(failed);
		await act(async () => {
			await currentModel.refresh();
		});
		for (const refresh of [
			() => currentModel.refreshWorktree("/repo"),
			() => currentModel.refreshRepository("/repo"),
		]) {
			await act(async () => {
				await refresh();
			});
			expect(screen.getByRole("alert")).toHaveTextContent("all offline");
			expect(screen.getByRole("alert").parentElement).toBe(
				container.querySelector(".overflow-y-auto"),
			);
		}
		mocks.invoke.mockResolvedValue(initial);
		await userEvent
			.setup()
			.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
		await waitFor(() =>
			expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
		);
		expect(screen.getByText("Running session")).toBe(session);
	});

	it("Rustの局所取得失敗をWorktree行に表示し別対象の成功では消さず全体更新で復旧する", async () => {
		const user = userEvent.setup();
		setup();
		const session = await screen.findByText("Running session");
		const refreshTree = async (worktreePath: string) => {
			await act(async () => {
				await currentModel.refreshWorktree(worktreePath);
			});
			await waitFor(() =>
				expect(mocks.invoke).toHaveBeenLastCalledWith("refresh_workspaces", {
					worktreePath,
				}),
			);
		};
		const failed = workspaceListSnapshot(tree);
		failed.repositories[0].worktrees[0].status = {
			loaded: true,
			state: "refreshFailed",
			error: "worktree offline",
		};
		mocks.invoke.mockResolvedValue(failed);
		await refreshTree("/repo");
		const alert = await screen.findByRole("alert");
		expect(alert).toHaveTextContent(
			"Refresh failed. Showing previous information",
		);
		expect(alert).toHaveTextContent("worktree offline");
		expect(alert.parentElement).toContainElement(
			screen.getByTestId("worktree-item-feature"),
		);
		expect(screen.getByText("Running session")).toBe(session);
		await refreshTree("/other");
		expect(screen.getByRole("alert")).toHaveTextContent("worktree offline");
		mocks.invoke.mockResolvedValue(workspaceListSnapshot(tree));
		await user.click(
			screen.getByRole("button", { name: "Refresh Workspaces" }),
		);
		await waitFor(() =>
			expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
		);
		expect(screen.getByText("Running session")).toBe(session);
	});

	it.each(["自動", "手動"])(
		"%s更新中・失敗中・復旧後も選択中Sessionと実行中Workflowを維持し中断操作を呼ばない",
		async (trigger) => {
			const user = userEvent.setup();
			const running = workspaceListSnapshot({
				...tree,
				nodes: [
					{
						kind: "sequence",
						id: "running-workflow",
						title: "Running workflow",
						status: "active",
						workflowCapabilities: { canAbort: true, canArchive: false },
						children: tree.nodes,
						updatedAt: 1,
					},
				],
			});
			mocks.invoke.mockResolvedValue(running);
			const { onSelectWorktree, onWorkspaceSelectionInvalidated } = setup();
			const workflow = await screen.findByRole("button", {
				name: "Running workflow",
			});
			const session = screen.getByRole("button", {
				name: "Running session, active",
			});
			const assertContinuing = () => {
				expect(screen.getByRole("button", { name: "Running workflow" })).toBe(
					workflow,
				);
				expect(
					screen.getByRole("button", { name: "Running session, active" }),
				).toBe(session);
				expect(session).toBeVisible();
				expect(session).toHaveAttribute("aria-current", "page");
				expect(within(workflow).getByTitle("active")).toBeVisible();
				expect(onSelectWorktree).not.toHaveBeenCalled();
				expect(onWorkspaceSelectionInvalidated).not.toHaveBeenCalled();
				expect(
					mocks.invoke.mock.calls.every(
						([command]) => command === "refresh_workspaces",
					),
				).toBe(true);
			};
			const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
			mocks.invoke.mockReturnValueOnce(pending.promise);
			const refresh = screen.getByRole("button", {
				name: "Refresh Workspaces",
			});
			if (trigger === "手動") {
				await user.click(refresh);
			} else {
				await act(async () => {
					mocks.listen.mock.calls.find(
						([name]) => name === "branch-list-sync",
					)?.[1]({ payload: null });
				});
			}
			expect(mocks.invoke).toHaveBeenCalledTimes(2);
			assertContinuing();
			const failed = structuredClone(running);
			failed.repositories[0].worktrees[0].status.error = "worktree failed";
			await act(async () => pending.resolve(failed));
			expect(screen.getByRole("alert")).toHaveTextContent(
				"Showing previous information",
			);
			assertContinuing();
			const recovery = deferred<ReturnType<typeof workspaceListSnapshot>>();
			mocks.invoke.mockReturnValueOnce(recovery.promise);
			await user.click(refresh);
			expect(refresh).toBeDisabled();
			assertContinuing();
			await act(async () => recovery.resolve(structuredClone(running)));
			expect(refresh).toBeEnabled();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			assertContinuing();
			await user.click(
				screen.getByRole("button", { name: "Open menu for Running workflow" }),
			);
			expect(screen.getByRole("menuitem", { name: "Abort" })).toBeEnabled();
		},
	);

	it("ボタンは一つで更新中と失敗後も展開・選択・スクロールを保持する", async () => {
		const user = userEvent.setup();
		const { container, onSelectWorktree } = setup();
		const worktree = await screen.findByTestId("worktree-item-feature");
		const node = screen.getByText("Running session");
		const scroll = container.querySelector(".overflow-y-auto") as HTMLElement;
		scroll.scrollTop = 42;
		const buttons = screen.getAllByRole("button", { name: /Refresh/ });
		expect(buttons).toHaveLength(1);
		expect(buttons[0]).toHaveAccessibleName("Refresh Workspaces");
		const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValueOnce(pending.promise);
		await user.click(buttons[0]);
		expect(buttons[0]).toBeDisabled();
		expect(buttons[0]).toHaveAttribute("aria-busy", "true");
		await user.click(buttons[0]);
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		expect(screen.getByText("Running session")).toBe(node);
		expect(worktree).toHaveAttribute("aria-expanded", "true");
		expect(scroll.scrollTop).toBe(42);
		const failed = workspaceListSnapshot(tree);
		failed.status = {
			loaded: true,
			state: "refreshFailed",
			error: "deadline exceeded",
		};
		await act(async () => pending.resolve(failed));
		expect(buttons[0]).toBeEnabled();
		expect(screen.getByRole("alert")).toHaveTextContent(
			"Showing previous information",
		);
		expect(screen.getByText("Running session")).toBe(node);
		await user.click(buttons[0]);
		await waitFor(() =>
			expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
		);
		expect(screen.getByText("Running session")).toBe(node);
		expect(scroll.scrollTop).toBe(42);
		expect(onSelectWorktree).not.toHaveBeenCalled();
		expect(
			mocks.invoke.mock.calls.every(([name]) => name === "refresh_workspaces"),
		).toBe(true);
	});

	it("自動更新失敗後もRepositoryとWorktreeの前回情報とエラーを表示し復旧する", async () => {
		const user = userEvent.setup();
		setup();
		await screen.findByTestId("worktree-item-feature");
		const stale = workspaceListSnapshot(tree);
		stale.repositories[0].status.error = "repository failed";
		stale.repositories[0].worktrees[0].status.error = "worktree failed";
		mocks.invoke.mockResolvedValueOnce(stale);
		await act(async () => {
			mocks.listen.mock.calls.find(
				([name]) => name === "branch-list-sync",
			)?.[1]({ payload: null });
		});
		expect(screen.getAllByRole("alert")).toHaveLength(2);
		for (const alert of screen.getAllByRole("alert"))
			expect(alert).toHaveTextContent("Showing previous information");
		expect(screen.getByText("Running session")).toBeVisible();
		await user.click(
			screen.getByRole("button", { name: "Refresh Workspaces" }),
		);
		await waitFor(() =>
			expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
		);
	});

	it("折りたたみ中も取得した子一覧を保持し展開時に表示する", async () => {
		const user = userEvent.setup();
		setup();
		await screen.findByTestId("worktree-item-feature");
		const repoButton = screen.getByRole("button", { name: "repo1" });
		await user.click(repoButton);
		const next = workspaceListSnapshot({
			...tree,
			nodes: [{ ...tree.nodes[0], title: "Updated session" }],
		});
		mocks.invoke.mockResolvedValueOnce(next);
		await user.click(
			screen.getByRole("button", { name: "Refresh Workspaces" }),
		);
		await user.click(repoButton);
		expect(screen.getByText("Updated session")).toBeVisible();
		expect(screen.getByTestId("worktree-item-feature")).toHaveAttribute(
			"aria-expanded",
			"true",
		);
	});

	it("初回取得中と失敗と正常な空を区別し失敗中も更新できる", async () => {
		const user = userEvent.setup();
		const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValueOnce(pending.promise);
		setup();
		expect(screen.getByRole("status")).toHaveTextContent("Loading Workspaces");
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		await act(async () =>
			pending.resolve({
				generation: 1,
				repositories: [],
				status: { loaded: false, state: "initialFailed", error: "offline" },
			}),
		);
		expect(screen.getByRole("alert")).toHaveTextContent("Initial load failed");
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		mocks.invoke.mockResolvedValueOnce({
			generation: 2,
			status: { loaded: true, error: null, state: "empty" },
			repositories: [],
		});
		await user.click(
			screen.getByRole("button", { name: "Refresh Workspaces" }),
		);
		expect(await screen.findByText("No Repository")).toBeVisible();
	});

	it.each(["repository", "worktree"] as const)(
		"%s階層の未取得・初回失敗・正常な空を区別する",
		async (level) => {
			const user = userEvent.setup();
			const initial = workspaceListSnapshot();
			if (level === "repository") {
				initial.repositories[0].status = {
					loaded: false,
					error: null,
					state: "loading",
				};
				initial.repositories[0].branches = [];
				initial.repositories[0].worktrees = [];
			} else {
				initial.repositories[0].worktrees[0].status = {
					loaded: false,
					state: "loading",
					error: null,
				};
				initial.repositories[0].worktrees[0].snapshot = null;
			}
			mocks.invoke.mockResolvedValueOnce(initial);
			setup();
			const loadingName =
				level === "repository"
					? "Loading worktrees"
					: "Loading sessions and workflows";
			expect(
				await screen.findByRole("status", { name: loadingName }),
			).toBeVisible();
			expect(screen.queryByText("No worktrees")).not.toBeInTheDocument();
			expect(
				screen.queryByText("No sessions or workflows"),
			).not.toBeInTheDocument();
			const failed = structuredClone(initial);
			const status =
				level === "repository"
					? failed.repositories[0].status
					: failed.repositories[0].worktrees[0].status;
			status.error = "first load failed";
			status.state = "initialFailed";
			mocks.invoke.mockResolvedValueOnce(failed);
			const button = screen.getByRole("button", { name: "Refresh Workspaces" });
			await user.click(button);
			expect(await screen.findByRole("alert")).toHaveTextContent(
				"Initial load failed",
			);
			expect(
				screen.queryByRole("status", { name: loadingName }),
			).not.toBeInTheDocument();
			expect(screen.queryByText("No worktrees")).not.toBeInTheDocument();
			expect(
				screen.queryByText("No sessions or workflows"),
			).not.toBeInTheDocument();
			const empty = workspaceListSnapshot();
			if (level === "repository") {
				empty.repositories[0].branches = [];
				empty.repositories[0].status.state = "empty";
				empty.repositories[0].worktrees = [];
			}
			mocks.invoke.mockResolvedValueOnce(empty);
			await user.click(button);
			expect(
				await screen.findByText(
					level === "repository" ? "No worktrees" : "No sessions or workflows",
				),
			).toBeVisible();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		},
	);

	it("正常な空と削除は反映する", async () => {
		const user = userEvent.setup();
		setup();
		await screen.findByTestId("worktree-item-feature");
		mocks.invoke.mockResolvedValueOnce(workspaceListSnapshot());
		await user.click(
			screen.getByRole("button", { name: "Refresh Workspaces" }),
		);
		expect(await screen.findByText("No sessions or workflows")).toBeVisible();
		expect(screen.queryByText("Running session")).not.toBeInTheDocument();
		const empty = workspaceListSnapshot();
		empty.repositories[0].branches = [];
		empty.repositories[0].status.state = "empty";
		empty.repositories[0].worktrees = [];
		mocks.invoke.mockResolvedValueOnce(empty);
		await user.click(
			screen.getByRole("button", { name: "Refresh Workspaces" }),
		);
		expect(await screen.findByText("No worktrees")).toBeVisible();
		expect(
			screen.queryByTestId("worktree-item-feature"),
		).not.toBeInTheDocument();
	});

	it("登録一覧の初回失敗を成功応答のstatusで表示し再取得で解消する", async () => {
		const user = userEvent.setup();
		mocks.invoke.mockResolvedValueOnce({
			generation: 1,
			status: {
				loaded: false,
				error: "repositories offline",
				state: "initialFailed",
			},
			repositories: [],
		});
		setup();
		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Initial load failed",
		);
		expect(screen.getByRole("alert")).toHaveTextContent("repositories offline");
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		expect(screen.queryByRole("status")).not.toBeInTheDocument();
		const button = screen.getByRole("button", { name: "Refresh Workspaces" });
		expect(button).toBeEnabled();
		await user.click(button);
		expect(await screen.findByText("Running session")).toBeVisible();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});

	it("登録一覧の更新失敗を成功応答のstatusで示し前回一覧を保持する", async () => {
		const user = userEvent.setup();
		setup();
		const node = await screen.findByText("Running session");
		const stale = workspaceListSnapshot(tree);
		stale.status.error = "repositories offline";
		mocks.invoke.mockResolvedValueOnce(stale);
		const button = screen.getByRole("button", { name: "Refresh Workspaces" });
		await user.click(button);
		expect(screen.getByRole("alert")).toHaveTextContent(
			"Refresh failed. Showing previous information",
		);
		expect(screen.getByRole("alert")).toHaveTextContent("repositories offline");
		expect(screen.getByText("Running session")).toBe(node);
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		expect(button).toBeEnabled();
		await user.click(button);
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(screen.getByText("Running session")).toBe(node);
	});

	it.each(["repository", "worktree", "all"])(
		"%sの通信失敗は取得失敗と混ぜず対象を通知し一覧を保持する",
		async (scope) => {
			const user = userEvent.setup();
			const initial = workspaceListSnapshot(tree);
			initial.repositories[0].status = {
				loaded: true,
				state: "refreshFailed",
				error: "scan failed",
			};
			mocks.invoke.mockResolvedValue(initial);
			setup();
			const session = await screen.findByText("Running session");
			mocks.invoke.mockRejectedValueOnce(new Error("connection lost"));
			await act(async () => {
				if (scope === "repository")
					await currentModel.refreshRepository("/repo");
				else if (scope === "worktree")
					await currentModel.refreshWorktree("/repo");
				else await currentModel.refresh();
			});
			expect(screen.getAllByRole("alert")).toHaveLength(2);
			expect(screen.getByText(/Could not confirm/)).toHaveTextContent(
				scope === "all"
					? "Could not confirm the refresh result."
					: "Could not confirm the refresh result for /repo.",
			);
			expect(screen.getByText(/Could not confirm/)).toHaveTextContent(
				"connection lost",
			);
			expect(screen.getByText(/scan failed/)).toHaveTextContent(
				"Showing previous information",
			);
			expect(screen.getByText("Running session")).toBe(session);
			await user.click(
				screen.getByRole("button", { name: "Refresh Workspaces" }),
			);
			expect(screen.queryByText(/Could not confirm/)).not.toBeInTheDocument();
			expect(screen.getByRole("alert")).toHaveTextContent("scan failed");
		},
	);

	it("初回の通信失敗は取得済みや空に分類せず再試行できる", async () => {
		mocks.invoke.mockRejectedValueOnce(new Error("offline"));
		setup();
		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Could not confirm the refresh result. offline",
		);
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		expect(screen.queryByRole("status")).not.toBeInTheDocument();
		await userEvent
			.setup()
			.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
		expect(await screen.findByText("Running session")).toBeVisible();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});

	it("利用者が折りたたんだWorktreeを更新中と失敗後と復旧後も維持する", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = setup();
		const worktree = await screen.findByTestId("worktree-item-feature");
		await user.click(worktree);
		expect(worktree).toHaveAttribute("aria-expanded", "false");
		const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValueOnce(pending.promise);
		const button = screen.getByRole("button", { name: "Refresh Workspaces" });
		await user.click(button);
		expect(worktree).toHaveAttribute("aria-expanded", "false");
		await act(async () => pending.reject(new Error("offline")));
		expect(worktree).toHaveAttribute("aria-expanded", "false");
		const updated = workspaceListSnapshot({
			...tree,
			nodes: [{ ...tree.nodes[0], title: "Updated session" }],
		});
		mocks.invoke.mockResolvedValueOnce(updated);
		await user.click(button);
		expect(screen.getByTestId("worktree-item-feature")).toBe(worktree);
		expect(worktree).toHaveAttribute("aria-expanded", "false");
		expect(onSelectWorktree).not.toHaveBeenCalled();
		await user.click(worktree);
		expect(screen.getByText("Updated session")).toBeVisible();
	});
});
