import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceList } from "@/hooks/useWorkspaceList";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { workspaceListSnapshot } from "@/test/workspaceList";
import type { WorkspaceTreeSnapshot } from "@/types/workspace-tree";
import { WorkspaceList as WorkspaceListView } from "./WorkspaceList";

function WorkspaceList(
	props: Omit<React.ComponentProps<typeof WorkspaceListView>, "model">,
) {
	const model = useWorkspaceList();
	return <WorkspaceListView {...props} model={model} />;
}

const states = stateSubscriptions();
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@/lib/client", () => ({
	invokeClient: mocks.invoke,
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
	firstState: (...args: Parameters<typeof states.firstState>) =>
		states.firstState(...args),
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

describe("Workspaces subscriptions", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		states.clear();
		states.publish("workspaces", workspaceListSnapshot(tree));
		mocks.invoke.mockResolvedValue(undefined);
		mocks.listen.mockResolvedValue(vi.fn());
	});
	it.each([
		["*", "review_comments_watch"],
		["/repo", "repository_scan"],
		["/repo/worktree", "repository_scan"],
		["session", "workflow_node_start"],
	])(
		"BackgroundFailuresへ一覧の対象%sを渡して要対応を表示する",
		async (target, operation) => {
			states.publish(
				"workspaces",
				workspaceListSnapshot(tree, "/repo/worktree"),
			);
			states.publish(
				{ kind: "failures", args: [target] },
				{
					requiresAttention: true,
					items: [
						{
							operation,
							target,
							classification: "StateRequired",
							message: `repair ${target}`,
							count: 1,
							firstObservedMs: 1,
							lastObservedMs: 1,
							requiresAttention: true,
						},
					],
				},
			);
			setup();
			await screen.findByText("Running session");
			expect(states.subscribeState).toHaveBeenCalledWith(
				{ kind: "failures", args: [target] },
				expect.any(Function),
				expect.any(Function),
			);
			expect(screen.getAllByText("要対応")).toHaveLength(1);
			fireEvent.click(screen.getByText("要対応"));
			const message = screen.getByText(`repair ${target}`);
			expect(message).toBeVisible();
			const row =
				target === "*"
					? screen.getByText("Workspaces").parentElement
					: target === "/repo"
						? screen.getByText("repo").closest("button")?.parentElement
								?.parentElement
						: target === "/repo/worktree"
							? screen.getByTestId("worktree-item-feature").parentElement
							: screen.getByRole("button", { name: "Running session, active" })
									.parentElement;
			expect(message.closest("details")?.parentElement).toBe(row);
		},
	);

	it.each(["all", "repository", "worktree"] as const)(
		"%sの取得失敗を購読から表示し前回の行・選択を保持する",
		async (scope) => {
			const { onSelectWorktree, onWorkspaceSelectionInvalidated } = setup();
			const node = await screen.findByText("Running session");
			const failed = workspaceListSnapshot(tree);
			const status =
				scope === "all"
					? failed.status
					: scope === "repository"
						? failed.repositories[0].status
						: failed.repositories[0].worktrees[0].status;
			Object.assign(status, {
				loaded: true,
				state: "refreshFailed",
				error: `${scope} offline`,
			});
			act(() => states.publish("workspaces", failed));
			const alert = screen.getByRole("alert");
			expect(alert).toHaveTextContent("Showing previous information");
			expect(alert).toHaveTextContent(`${scope} offline`);
			expect(screen.getByText("Running session")).toBe(node);
			expect(onSelectWorktree).not.toHaveBeenCalled();
			expect(onWorkspaceSelectionInvalidated).not.toHaveBeenCalled();
			expect(mocks.invoke).not.toHaveBeenCalled();
			act(() => states.publish("workspaces", workspaceListSnapshot(tree)));
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(screen.getByText("Running session")).toBe(node);
		},
	);
	it("Refreshボタンは一つで操作は状態を返さず更新中も展開・スクロールを保持する", async () => {
		const user = userEvent.setup();
		const { container } = setup();
		const node = await screen.findByText("Running session");
		const scroll = container.querySelector(".overflow-y-auto") as HTMLElement;
		scroll.scrollTop = 42;
		const buttons = screen.getAllByRole("button", { name: /Refresh/ });
		expect(buttons).toHaveLength(1);
		const request = deferred<void>();
		mocks.invoke.mockReturnValueOnce(request.promise);
		await user.click(buttons[0]);
		expect(buttons[0]).toBeDisabled();
		expect(buttons[0]).toHaveAttribute("aria-busy", "true");
		await user.click(buttons[0]);
		expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
			"refresh_workspaces",
			{},
		);
		expect(screen.getByText("Running session")).toBe(node);
		expect(scroll.scrollTop).toBe(42);
		await act(async () => request.resolve());
		expect(buttons[0]).toBeEnabled();
		expect(screen.getByText("Running session")).toBe(node);
		act(() =>
			states.publish(
				"workspaces",
				workspaceListSnapshot({
					...tree,
					nodes: [{ ...tree.nodes[0], title: "Updated session" }],
				}),
			),
		);
		expect(screen.getByText("Updated session")).toBeVisible();
		expect(scroll.scrollTop).toBe(42);
	});
	it("初回の取得中・取得失敗・空を区別し失敗後も手動更新できる", async () => {
		states.clear();
		setup();
		expect(screen.getByRole("status")).toHaveTextContent("Loading Workspaces");
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		act(() =>
			states.publish("workspaces", {
				generation: 1,
				repositories: [],
				status: { loaded: false, state: "initialFailed", error: "offline" },
			}),
		);
		expect(screen.getByRole("alert")).toHaveTextContent("Initial load failed");
		expect(screen.queryByText("No Repository")).not.toBeInTheDocument();
		await userEvent
			.setup()
			.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
		expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
			"refresh_workspaces",
			{},
		);
		act(() =>
			states.publish("workspaces", {
				generation: 2,
				repositories: [],
				status: { loaded: true, state: "empty", error: null },
			}),
		);
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(screen.getByText("No Repository")).toBeVisible();
	});
	it.each(["repository", "worktree"] as const)(
		"%sを折りたたんでいても購読更新と失敗・復旧を反映できる",
		async (scope) => {
			const user = userEvent.setup();
			setup();
			await screen.findByText("Running session");
			const button =
				scope === "repository"
					? screen.getByRole("button", { name: "repo1" })
					: screen.getByTestId("worktree-item-feature");
			await user.click(button);
			const next = workspaceListSnapshot({
				...tree,
				nodes: [{ ...tree.nodes[0], title: "Updated session" }],
			});
			act(() => states.publish("workspaces", next));
			if (scope === "repository")
				expect(screen.getByText("Updated session")).not.toBeVisible();
			else
				expect(screen.queryByText("Updated session")).not.toBeInTheDocument();
			await user.click(button);
			expect(screen.getByText("Updated session")).toBeVisible();
		},
	);
	it("操作の通信失敗でも一覧を保持し同じボタンで再要求できる", async () => {
		setup();
		const node = await screen.findByText("Running session");
		mocks.invoke.mockRejectedValueOnce(new Error("offline"));
		await userEvent
			.setup()
			.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
		expect(screen.getByRole("alert")).toHaveTextContent(
			"Could not confirm the refresh result. offline",
		);
		expect(screen.getByText("Running session")).toBe(node);
		await userEvent
			.setup()
			.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});
	it("購読から届いた正常な削除を表示に反映する", async () => {
		setup();
		await screen.findByText("Running session");
		act(() =>
			states.publish("workspaces", {
				generation: 2,
				repositories: [],
				status: { loaded: true, state: "empty", error: null },
			}),
		);
		expect(screen.queryByText("Running session")).not.toBeInTheDocument();
		expect(screen.getByText("No Repository")).toBeVisible();
	});
});
