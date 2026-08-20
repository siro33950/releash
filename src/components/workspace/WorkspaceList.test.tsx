import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceTreeReconciliationEvent } from "@/hooks/useWorkspaceTreeNodes";
import type { AgentSessionItem } from "@/types/agent-session";
import type { WorktreeBranch } from "@/types/git";
import type {
	CenterSelection,
	WorkspaceTreeItem,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { WorkspaceList } from "./WorkspaceList";

type MockWorkspaceTreeState = {
	nodes: WorkspaceTreeItem[];
	sessions?: AgentSessionItem[];
	preferredNodeId?: string | null;
	workflowHistory?: WorkspaceWorkflowHistoryItem[];
	reconciliationEvent?: WorkspaceTreeReconciliationEvent | null;
	loading?: boolean;
	error?: string | null;
};

const mocks = vi.hoisted(() => ({
	invoke: vi.fn().mockResolvedValue(null),
	emit: vi.fn().mockResolvedValue(undefined),
	listen: vi.fn().mockResolvedValue(() => {}),
	openUrl: vi.fn().mockResolvedValue(undefined),
	refreshTree: vi.fn().mockResolvedValue(undefined),
	beginArchiveReconciliation: vi.fn().mockResolvedValue(undefined),
	synchronizeSelectedNodeId: vi.fn(),
	isReconciliationEventCurrent: vi.fn().mockReturnValue(true),
	refreshWorktrees: vi.fn().mockResolvedValue(undefined),
	treeStateOverrides: new Map<string, MockWorkspaceTreeState>(),
	selectedNodeIds: new Map<string, string | null>(),
	worktreeBranches: [] as WorktreeBranch[],
	cleanupCandidates: [] as WorktreeBranch[],
}));

vi.mock("react-resizable-panels", () => ({
	Panel: ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	),
	Group: ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	),
	Separator: () => <div />,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
	emit: mocks.emit,
	listen: mocks.listen,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("@/hooks/useWorkflowConfig", () => ({
	useWorkflowConfig: () => ({
		workflows: [{ name: "release", description: "Release workflow" }],
		loading: false,
		error: null,
	}),
}));
vi.mock("@/hooks/useWorkspaceTreeNodes", () => ({
	useWorkspaceTreeNodes: (worktreePath: string) => {
		const state = mocks.treeStateOverrides.get(worktreePath) ?? {
			nodes: [],
		};
		const [sessions, setSessions] = useState<AgentSessionItem[]>(
			state.sessions ?? [],
		);
		useEffect(() => {
			if (state.sessions) {
				setSessions(state.sessions);
				return;
			}
			let active = true;
			void mocks
				.invoke("list_workspace_worktree_nodes", { worktreePath })
				.then((snapshot: unknown) => {
					if (!active) return;
					setSessions(
						(snapshot as { sessions?: AgentSessionItem[] } | null)?.sessions ??
							[],
					);
				});
			return () => {
				active = false;
			};
		}, [state.sessions, worktreePath]);
		return {
			nodes: state.nodes,
			sessions,
			preferredNodeId: state.preferredNodeId ?? null,
			workflowHistory: state.workflowHistory ?? [],
			reconciliationEvent: state.reconciliationEvent ?? null,
			loading: state.loading ?? false,
			error: state.error ?? null,
			refresh: mocks.refreshTree,
			beginArchiveReconciliation: mocks.beginArchiveReconciliation,
			synchronizeSelectedNodeId: (selectedNodeId: string | null) => {
				mocks.selectedNodeIds.set(worktreePath, selectedNodeId);
				mocks.synchronizeSelectedNodeId(selectedNodeId);
			},
			isReconciliationEventCurrent: mocks.isReconciliationEventCurrent,
		};
	},
}));
vi.mock("@/hooks/useWorktreeList", () => ({
	useWorktreeList: () => ({
		branches: mocks.worktreeBranches,
		cleanupCandidates: mocks.cleanupCandidates,
		loading: false,
		refresh: mocks.refreshWorktrees,
	}),
}));

const directNode: WorkspaceTreeItem = {
	kind: "node",
	id: "4f168b74-f9cf-4d51-9970-81ea281bc983",
	title: "Direct session",
	status: "running",
	contentKind: "session",
	capabilities: { canApprove: false, canRetry: false, canClose: true },
	updatedAt: 1,
};

const recursiveTree: WorkspaceTreeItem[] = [
	directNode,
	{
		kind: "workflow",
		id: "workflow-internal-uuid",
		title: "Release workflow",
		status: "running",
		capabilities: {
			canStop: true,
			canResume: false,
			canAbort: true,
			canArchive: false,
		},
		updatedAt: 2,
		children: [
			{
				kind: "node",
				id: "workflow-session-internal-uuid",
				title: "Prepare",
				status: "completed",
				contentKind: "session",
				capabilities: { canApprove: false, canRetry: false, canClose: false },
				updatedAt: 3,
			},
			{
				kind: "fanout",
				id: "fanout-internal-uuid",
				title: "Review all",
				status: "running",
				updatedAt: 4,
				children: [
					{
						kind: "node",
						id: "fanout-child-internal-uuid",
						title: "Architecture review",
						status: "running",
						contentKind: "command",
						capabilities: {
							canApprove: false,
							canRetry: false,
							canClose: false,
						},
						updatedAt: 5,
					},
				],
			},
		],
	},
];

function makeBranch(): WorktreeBranch {
	return {
		name: "feature",
		is_main_worktree: false,
		worktree_path: "/repo/wt",
		dirty_count: 0,
		is_merged: false,
		ahead: 0,
		behind: 0,
		has_upstream: false,
		base_ahead: 0,
		management_kind: "working_area",
	};
}

function renderWorkspaceList(
	overrides: Partial<React.ComponentProps<typeof WorkspaceList>> = {},
) {
	const onSelectWorktree = vi.fn();
	const result = render(
		<WorkspaceList
			repoPaths={["/repo"]}
			selectedRootPath="/repo/wt"
			centerSelection={null}
			onSelectWorktree={onSelectWorktree}
			onAddRepo={vi.fn()}
			onShowSettings={vi.fn()}
			{...overrides}
		/>,
	);
	const rerenderWorkspaceList = (
		nextOverrides: Partial<React.ComponentProps<typeof WorkspaceList>> = {},
	) => {
		result.rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={null}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
				{...overrides}
				{...nextOverrides}
			/>,
		);
	};
	return {
		...result,
		onSelectWorktree,
		rerenderWorkspaceList,
	};
}

function mockDeferredProviderCreate() {
	const deferred: {
		resolve?: (agentSessionId: string) => void;
		reject?: (error: Error) => void;
	} = {};
	mocks.invoke.mockImplementation((command: string) => {
		if (command === "list_workspace_worktree_nodes") {
			return Promise.resolve({ nodes: [], sessions: [] });
		}
		if (command === "list_available_agent_session_providers") {
			return Promise.resolve(["codex"]);
		}
		if (command === "create_agent_session") {
			return new Promise<string>((resolve, reject) => {
				deferred.resolve = resolve;
				deferred.reject = reject;
			});
		}
		return Promise.resolve(null);
	});
	return deferred;
}

async function launchProviderCreate(
	user: ReturnType<typeof userEvent.setup>,
	onSelectWorktree: ReturnType<typeof vi.fn>,
) {
	await user.click(screen.getByRole("button", { name: "Create in feature" }));
	await user.hover(screen.getByRole("menuitem", { name: "NewSession" }));
	const provider = await screen.findByRole("menuitem", { name: "codex" });
	act(() => provider.focus());
	await user.keyboard("{Enter}");
	await waitFor(() => {
		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			expect.objectContaining({ kind: "agent_session_launching" }),
		);
	});
}

function wireSelectionRoundTrip({
	onSelectWorktree,
	rerenderWorkspaceList,
}: {
	onSelectWorktree: ReturnType<typeof vi.fn>;
	rerenderWorkspaceList: (
		nextOverrides?: Partial<React.ComponentProps<typeof WorkspaceList>>,
	) => void;
}) {
	onSelectWorktree.mockImplementation(
		(
			_rootPath: string,
			_branchName?: string,
			_repoName?: string,
			selection?: CenterSelection,
		) => {
			if (selection) {
				rerenderWorkspaceList({ centerSelection: selection });
			}
		},
	);
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) {
		if (typeof mock === "function" && "mockClear" in mock) {
			mock.mockClear();
		}
	}
	mocks.worktreeBranches = [makeBranch()];
	mocks.treeStateOverrides.clear();
	mocks.selectedNodeIds.clear();
	mocks.cleanupCandidates = [];
	mocks.treeStateOverrides.set("/repo/wt", { nodes: recursiveTree });
	mocks.invoke.mockResolvedValue(null);
	mocks.refreshTree.mockResolvedValue(undefined);
	mocks.beginArchiveReconciliation.mockResolvedValue(undefined);
	mocks.isReconciliationEventCurrent.mockImplementation(
		(event: WorkspaceTreeReconciliationEvent, selectedNodeId: string | null) =>
			event.requestContext.worktreePath === "/repo/wt" &&
			event.requestContext.selectedNodeId === selectedNodeId,
	);
});

describe("WorkspaceList", () => {
	it("掃除候補を操作なしの別sectionに表示する", () => {
		mocks.worktreeBranches = [makeBranch()];
		mocks.cleanupCandidates = [
			{
				...makeBranch(),
				name: "released",
				worktree_path: "/repo/released",
				management_kind: "cleanup_candidate",
			},
			{
				...makeBranch(),
				name: "orphan",
				worktree_path: "/repo/orphan",
				management_kind: "untracked_cleanup_candidate",
			},
		];

		renderWorkspaceList();

		const section = screen.getByLabelText("掃除候補");
		expect(within(section).getByText("released")).toBeInTheDocument();
		expect(within(section).getByText("orphan")).toBeInTheDocument();
		expect(within(section).getByText("/repo/released")).toBeInTheDocument();
		expect(within(section).getByText("/repo/orphan")).toBeInTheDocument();
		expect(within(section).getByText("台帳外・掃除候補")).toBeInTheDocument();
		expect(within(section).queryByRole("button")).not.toBeInTheDocument();
	});

	it("renders the backend-owned recursive Workflow and Fanout hierarchy", () => {
		const { container } = renderWorkspaceList();

		expect(screen.getByText("Release workflow")).toBeInTheDocument();
		expect(screen.getByText("Review all")).toBeInTheDocument();
		expect(screen.getByText("Architecture review")).toBeInTheDocument();
		expect(
			screen
				.getByRole("button", { name: "Release workflow" })
				.querySelector("svg.lucide-workflow"),
		).toBeInTheDocument();
		expect(
			screen
				.getByRole("button", { name: "Review all" })
				.querySelector("svg.lucide-git-fork"),
		).toBeInTheDocument();
		expect(container.querySelectorAll("svg.lucide-git-fork")).toHaveLength(1);
	});

	it("backendが絞り込んだStandalone AgentSessionを選択できる", async () => {
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({
					nodes: [],
					sessions: [
						{
							id: "provider-agent-1",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "claude",
							lifecycle: "open",
							activity: "idle",
							lastExitAbnormal: false,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
					],
				});
			}
			return Promise.resolve(null);
		});
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(
			await screen.findByRole("button", {
				name: "Claude AgentSession, open",
			}),
		);

		expect(mocks.invoke).toHaveBeenCalledWith("list_workspace_worktree_nodes", {
			worktreePath: "/repo/wt",
		});
		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "agent_session",
				worktreePath: "/repo/wt",
				agentSessionId: "provider-agent-1",
			},
		);
	});

	it("Archived AgentSessionをWorkspaceからSessionHistoryへ移して復帰できる", async () => {
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({
					nodes: [],
					sessions: [
						{
							id: "provider-agent-archived",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "claude",
							lifecycle: "archived",
							activity: "idle",
							lastExitAbnormal: false,
							operations: {
								canArchive: false,
								canRestore: true,
								canDelete: true,
							},
						},
					],
				});
			}
			if (command === "list_agent_session_history") {
				return Promise.resolve({ items: [], nextAfter: null });
			}
			if (command === "restore_agent_session") {
				return Promise.resolve("restored");
			}
			return Promise.resolve(null);
		});
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		expect(
			screen.queryByRole("button", {
				name: "Claude AgentSession, archived",
			}),
		).toBeNull();
		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.hover(screen.getByRole("menuitem", { name: "SessionHistory" }));
		const archived = await screen.findByRole("menuitem", {
			name: /Claude AgentSession/,
		});
		expect(
			screen.getByRole("button", { name: "Delete Claude AgentSession" }),
		).toBeVisible();
		act(() => archived.focus());
		await user.keyboard("{Enter}");

		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"restore_agent_session",
				expect.objectContaining({
					agentSessionId: "provider-agent-archived",
					rows: 24,
					cols: 80,
					callerRequestId: expect.any(String),
				}),
			);
			expect(onSelectWorktree).toHaveBeenCalledWith(
				"/repo/wt",
				"feature",
				"repo",
				{
					kind: "agent_session",
					worktreePath: "/repo/wt",
					agentSessionId: "provider-agent-archived",
				},
			);
		});
	});

	it("Standalone AgentSessionの実行状態はテキストでなくアイコン色で表現する", async () => {
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({
					nodes: [],
					sessions: [
						{
							id: "provider-agent-running",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "claude",
							lifecycle: "open",
							activity: "running",
							lastExitAbnormal: false,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
						{
							id: "provider-agent-open",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "claude",
							lifecycle: "open",
							activity: "idle",
							lastExitAbnormal: false,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
						{
							id: "provider-agent-paused-abnormal",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "codex",
							lifecycle: "paused",
							activity: "idle",
							lastExitAbnormal: true,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
						{
							id: "provider-agent-paused",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "codex",
							lifecycle: "paused",
							activity: "idle",
							lastExitAbnormal: false,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
					],
				});
			}
			return Promise.resolve(undefined);
		});
		renderWorkspaceList();

		const runningRow = await screen.findByRole("button", {
			name: "Claude AgentSession, running",
		});
		const openRow = screen.getByRole("button", {
			name: "Claude AgentSession, open",
		});
		const abnormalRow = screen.getByRole("button", {
			name: "Codex AgentSession, paused (exited abnormally)",
		});
		const pausedRow = screen.getByRole("button", {
			name: "Codex AgentSession, paused",
		});

		expect(within(runningRow).queryByText("running")).toBeNull();
		expect(within(openRow).queryByText("open")).toBeNull();
		expect(within(abnormalRow).queryByText(/paused/)).toBeNull();
		expect(within(pausedRow).queryByText("paused")).toBeNull();
		expect(within(runningRow).getByTitle("running").firstChild).toHaveClass(
			"text-blue-600",
			"dark:text-blue-300",
			"animate-pulse",
		);
		const openIcon = within(openRow).getByTitle("open").firstChild;
		expect(openIcon).toHaveClass("text-foreground");
		expect(openIcon).not.toHaveClass("animate-pulse");
		expect(
			within(abnormalRow).getByTitle("paused (exited abnormally)").firstChild,
		).toHaveClass("text-destructive");
		expect(within(pausedRow).getByTitle("paused").firstChild).toHaveClass(
			"text-muted-foreground",
		);
	});

	it("Standalone AgentSessionのXはArchiveしID不明時はDelete確認を要求する", async () => {
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({
					nodes: [],
					sessions: [
						{
							id: "provider-agent-unknown",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "claude",
							lifecycle: "open",
							activity: "idle",
							lastExitAbnormal: false,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
					],
				});
			}
			if (command === "archive_agent_session") {
				return Promise.resolve("delete_confirmation_required");
			}
			return Promise.resolve(undefined);
		});
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(
			await screen.findByRole("button", {
				name: "Archive Claude AgentSession",
			}),
		);

		expect(
			await screen.findByText(
				/This AgentSession has no Provider session ID and cannot be archived/,
			),
		).toBeVisible();
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"confirm_agent_session_archive_delete",
			expect.anything(),
		);

		await user.click(screen.getByRole("button", { name: "Delete" }));
		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"confirm_agent_session_archive_delete",
				expect.objectContaining({
					agentSessionId: "provider-agent-unknown",
				}),
			);
		});
	});

	it("Standalone AgentSessionのArchive成功を同じworktreeの表示へ通知する", async () => {
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({
					nodes: [],
					sessions: [
						{
							id: "provider-agent-known",
							workspaceIdentity: "/repo/wt",
							worktreePath: "/repo/wt",
							provider: "claude",
							lifecycle: "open",
							activity: "idle",
							lastExitAbnormal: false,
							operations: {
								canArchive: true,
								canRestore: false,
								canDelete: false,
							},
						},
					],
				});
			}
			if (command === "archive_agent_session") {
				return Promise.resolve("archived");
			}
			return Promise.resolve(undefined);
		});
		const refresh = vi.fn();
		window.addEventListener("agent-session-refresh", refresh);
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(
			await screen.findByRole("button", {
				name: "Archive Claude AgentSession",
			}),
		);

		await waitFor(() => expect(refresh).toHaveBeenCalledOnce());
		const event = refresh.mock.calls[0]?.[0];
		expect((event as CustomEvent).detail).toEqual({
			worktreePath: "/repo/wt",
		});
		window.removeEventListener("agent-session-refresh", refresh);
	});

	it("Standalone AgentSession一覧を共通snapshotから表示する", async () => {
		mocks.invoke.mockImplementation((command: string) => {
			if (command !== "list_workspace_worktree_nodes") {
				return Promise.resolve(null);
			}
			return Promise.resolve({
				nodes: [],
				sessions: [
					{
						id: "provider-agent-2",
						workspaceIdentity: "/repo/wt",
						worktreePath: "/repo/wt",
						provider: "codex",
						lifecycle: "open",
						activity: "idle",
						lastExitAbnormal: false,
						operations: {
							canArchive: true,
							canRestore: false,
							canDelete: false,
						},
					},
					{
						id: "provider-agent-1",
						workspaceIdentity: "/repo/wt",
						worktreePath: "/repo/wt",
						provider: "claude",
						lifecycle: "open",
						activity: "idle",
						lastExitAbnormal: false,
						operations: {
							canArchive: true,
							canRestore: false,
							canDelete: false,
						},
					},
				],
			});
		});
		renderWorkspaceList();

		expect(
			await screen.findByRole("button", {
				name: "Codex AgentSession, open",
			}),
		).toBeVisible();
		expect(mocks.invoke).toHaveBeenCalledWith("list_workspace_worktree_nodes", {
			worktreePath: "/repo/wt",
		});
	});

	it("renders an empty backend-owned Workflow branch without Node leaves", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				{
					kind: "workflow",
					id: "empty-workflow",
					title: "Empty workflow",
					status: "running",
					capabilities: {
						canStop: true,
						canResume: false,
						canAbort: true,
						canArchive: false,
					},
					children: [],
					updatedAt: 1,
				},
			],
			preferredNodeId: null,
		});

		renderWorkspaceList();

		expect(
			screen.getByRole("button", { name: "Empty workflow" }),
		).toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: /Direct session/ }),
		).not.toBeInTheDocument();
	});

	it("keeps one familiar content icon per Node and styles it from backend status", () => {
		renderWorkspaceList();

		const sessionRow = screen.getByRole("button", {
			name: "Direct session, running",
		});
		const sessionIcons = sessionRow.querySelectorAll("svg");
		expect(sessionIcons).toHaveLength(1);
		expect(sessionIcons[0]).toHaveClass(
			"lucide-bot",
			"text-blue-600",
			"animate-pulse",
		);

		const commandRow = screen.getByRole("button", {
			name: "Architecture review, running",
		});
		const commandIcons = commandRow.querySelectorAll("svg");
		expect(commandIcons).toHaveLength(1);
		expect(commandIcons[0]).toHaveClass(
			"lucide-terminal",
			"text-blue-600",
			"animate-pulse",
		);
	});

	it("shows the backend error reason on an errored session badge", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				{
					...directNode,
					status: "error",
					errorReason: "app server stopped",
				},
			],
		});

		renderWorkspaceList();

		expect(screen.getByTitle("app server stopped")).toBeInTheDocument();
	});

	it("toggles Workflow and Fanout branches without changing selection", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByRole("button", { name: "Release workflow" }));
		expect(screen.queryByText("Prepare")).not.toBeInTheDocument();
		expect(onSelectWorktree).not.toHaveBeenCalled();

		await user.click(screen.getByRole("button", { name: "Release workflow" }));
		await user.click(screen.getByRole("button", { name: "Review all" }));
		expect(screen.queryByText("Architecture review")).not.toBeInTheDocument();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("emits only an opaque Node selection from a leaf", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: /Architecture review/ }),
		);

		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: "fanout-child-internal-uuid",
			},
		);
	});

	it("does not render attempts, fanout coordinates, raw kinds, or internal ids", () => {
		renderWorkspaceList();

		expect(screen.queryByText(/attempt/i)).not.toBeInTheDocument();
		expect(screen.queryByText(/item \d/i)).not.toBeInTheDocument();
		expect(screen.queryByText(/child \d/i)).not.toBeInTheDocument();
		expect(
			screen.queryByText("workflow-internal-uuid"),
		).not.toBeInTheDocument();
		expect(
			screen.queryByText("fanout-child-internal-uuid"),
		).not.toBeInTheDocument();
		expect(screen.queryByText("command")).not.toBeInTheDocument();
	});

	it("uses preferredNodeId once for the initial selected Worktree", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: directNode.id,
		});
		const { onSelectWorktree, rerender } = renderWorkspaceList({
			autoSelectPreferredNode: true,
		});

		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(1));
		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: directNode.id,
			},
		);

		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: "fanout-child-internal-uuid",
		});
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={null}
				autoSelectPreferredNode={true}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);
		expect(onSelectWorktree).toHaveBeenCalledTimes(1);
	});

	it("re-arms preferred selection after auto selection is disabled", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: directNode.id,
		});
		const { onSelectWorktree, rerender } = renderWorkspaceList({
			autoSelectPreferredNode: true,
		});
		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(1));

		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: "fanout-child-internal-uuid",
		});
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={{
					kind: "node",
					worktreePath: "/repo/wt",
					nodeId: directNode.id,
				}}
				autoSelectPreferredNode={false}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);
		expect(onSelectWorktree).toHaveBeenCalledTimes(1);

		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={null}
				autoSelectPreferredNode={true}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);

		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(2));
		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: "fanout-child-internal-uuid",
			},
		);
	});

	it("keeps initial selection eligible while an empty snapshot has no preferred Node", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [],
			preferredNodeId: null,
		});
		const { onSelectWorktree, rerender } = renderWorkspaceList({
			autoSelectPreferredNode: true,
		});
		expect(onSelectWorktree).not.toHaveBeenCalled();

		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: directNode.id,
		});
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={null}
				autoSelectPreferredNode={true}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);

		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(1));
		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: directNode.id,
			},
		);
	});

	it("resets the preferred selection guard when the same branch gets a new Worktree path", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: directNode.id,
		});
		const { onSelectWorktree, rerender } = renderWorkspaceList({
			autoSelectPreferredNode: true,
		});
		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(1));

		mocks.worktreeBranches = [{ ...makeBranch(), worktree_path: null }];
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath={null}
				centerSelection={null}
				autoSelectPreferredNode={true}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);

		const recreatedNode = { ...directNode, id: "recreated-node" };
		mocks.worktreeBranches = [
			{ ...makeBranch(), worktree_path: "/repo/wt-recreated" },
		];
		mocks.treeStateOverrides.set("/repo/wt-recreated", {
			nodes: [recreatedNode],
			preferredNodeId: recreatedNode.id,
		});
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt-recreated"
				centerSelection={null}
				autoSelectPreferredNode={true}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);

		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(2));
		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt-recreated",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt-recreated",
				nodeId: recreatedNode.id,
			},
		);
	});

	it("does not apply a preferred Node while auto selection is disabled", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			preferredNodeId: directNode.id,
		});
		const { onSelectWorktree } = renderWorkspaceList({
			autoSelectPreferredNode: false,
		});

		await Promise.resolve();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("keeps a stable Node selected when the tree snapshot is replaced", () => {
		const selection = {
			kind: "node" as const,
			worktreePath: "/repo/wt",
			nodeId: "fanout-child-internal-uuid",
		};
		const { rerender } = renderWorkspaceList({ centerSelection: selection });

		expect(
			screen.getByRole("button", { name: /Architecture review/ }),
		).toHaveAttribute("aria-current", "page");

		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree.map((item) => ({ ...item, updatedAt: 99 })),
		});
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={selection}
				onSelectWorktree={vi.fn()}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);
		expect(
			screen.getByRole("button", { name: /Architecture review/ }),
		).toHaveAttribute("aria-current", "page");
	});

	it("passes only the Worktree-scoped selected opaque ID to the tree read", () => {
		renderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/other",
				nodeId: "foreign-node",
			},
		});

		expect(mocks.selectedNodeIds.get("/repo/wt")).toBeNull();
	});

	it("keeps occurrence order and the selected past occurrence when later executions append", async () => {
		const user = userEvent.setup();
		const occurrenceA1: WorkspaceTreeItem = {
			kind: "node",
			id: "occurrence-a-1",
			title: "A",
			status: "completed",
			contentKind: "session",
			capabilities: { canApprove: false, canRetry: false, canClose: false },
			updatedAt: 1,
		};
		const occurrenceB: WorkspaceTreeItem = {
			...occurrenceA1,
			id: "occurrence-b-1",
			title: "B",
			updatedAt: 2,
		};
		const occurrenceA2: WorkspaceTreeItem = {
			...occurrenceA1,
			id: "occurrence-a-2",
			updatedAt: 3,
		};
		const occurrenceC: WorkspaceTreeItem = {
			...occurrenceA1,
			id: "occurrence-c-1",
			title: "C",
			status: "running",
			updatedAt: 4,
		};
		const workflow = (children: WorkspaceTreeItem[]): WorkspaceTreeItem => ({
			kind: "workflow",
			id: "loop-workflow",
			title: "Loop workflow",
			status: "running",
			capabilities: {
				canStop: true,
				canResume: false,
				canAbort: true,
				canArchive: false,
			},
			updatedAt: 4,
			children,
		});
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [workflow([occurrenceA1, occurrenceB])],
		});
		const selection = {
			kind: "node" as const,
			worktreePath: "/repo/wt",
			nodeId: occurrenceA1.id,
		};
		const { onSelectWorktree, rerender } = renderWorkspaceList({
			centerSelection: selection,
		});

		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [workflow([occurrenceA1, occurrenceB, occurrenceA2, occurrenceC])],
		});
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath="/repo/wt"
				centerSelection={selection}
				onSelectWorktree={onSelectWorktree}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);

		const executionLabels = screen
			.getAllByRole("button")
			.map((button) => button.getAttribute("aria-label"))
			.filter((label) => label?.match(/^[ABC],/));
		expect(executionLabels).toEqual([
			"A, completed",
			"B, completed",
			"A, completed",
			"C, running",
		]);
		const [firstA, secondA] = screen.getAllByRole("button", {
			name: /^A, completed$/,
		});
		expect(firstA).toHaveAttribute("aria-current", "page");
		expect(secondA).not.toHaveAttribute("aria-current");

		await user.click(secondA);
		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: "occurrence-a-2",
			},
		);
	});

	it("NewSessionはNewWorkflowと同じsubmenuでProviderを選択して作成する", async () => {
		const user = userEvent.setup();
		let providerSessionListCalls = 0;
		const refreshAfterCreate = new Promise(() => {});
		const initialAttachment = {
			agentSessionId: "agent-session-1",
			workspaceIdentity: "/repo/wt",
			worktreePath: "/repo/wt",
			provider: "codex" as const,
		};
		mocks.invoke.mockImplementation((command) => {
			if (command === "list_workspace_worktree_nodes") {
				providerSessionListCalls += 1;
				return providerSessionListCalls === 1
					? Promise.resolve({ nodes: [], sessions: [] })
					: refreshAfterCreate;
			}
			if (command === "list_available_agent_session_providers") {
				return Promise.resolve(["codex"]);
			}
			if (command === "create_agent_session") {
				return Promise.resolve("agent-session-1");
			}
			return Promise.resolve(null);
		});
		const { onSelectWorktree, rerenderWorkspaceList } = renderWorkspaceList();
		wireSelectionRoundTrip({ onSelectWorktree, rerenderWorkspaceList });
		await waitFor(() => {
			expect(providerSessionListCalls).toBe(1);
		});

		await user.click(screen.getByRole("button", { name: "Create in feature" }));
		await user.hover(screen.getByRole("menuitem", { name: "NewSession" }));
		const provider = await screen.findByRole("menuitem", { name: "codex" });
		act(() => provider.focus());
		await user.keyboard("{Enter}");

		expect(
			screen.queryByRole("dialog", { name: "New AgentSession" }),
		).toBeNull();
		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"create_agent_session",
				expect.objectContaining({
					workspaceIdentity: "/repo/wt",
					worktreePath: "/repo/wt",
					provider: "codex",
					rows: 24,
					cols: 80,
					callerRequestId: expect.any(String),
				}),
			);
			expect(onSelectWorktree).toHaveBeenCalledWith(
				"/repo/wt",
				"feature",
				"repo",
				{
					kind: "agent_session",
					worktreePath: "/repo/wt",
					agentSessionId: "agent-session-1",
					initialAttachment,
				},
			);
		});
	});

	it("AgentSession作成のpending中に別Nodeへ移動した場合は作成成功でも選択を奪わない", async () => {
		const user = userEvent.setup();
		const createCall = mockDeferredProviderCreate();
		const { onSelectWorktree, rerenderWorkspaceList } = renderWorkspaceList();
		wireSelectionRoundTrip({ onSelectWorktree, rerenderWorkspaceList });

		await launchProviderCreate(user, onSelectWorktree);
		onSelectWorktree.mockClear();
		rerenderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: directNode.id,
			},
		});

		await act(async () => {
			createCall.resolve?.("agent-session-1");
		});

		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("AgentSession作成のpending中に別Nodeへ移動した場合は失敗しても選択を奪わずエラーを表示する", async () => {
		const user = userEvent.setup();
		const createCall = mockDeferredProviderCreate();
		const { onSelectWorktree, rerenderWorkspaceList } = renderWorkspaceList();
		wireSelectionRoundTrip({ onSelectWorktree, rerenderWorkspaceList });

		await launchProviderCreate(user, onSelectWorktree);
		onSelectWorktree.mockClear();
		rerenderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: directNode.id,
			},
		});

		await act(async () => {
			createCall.reject?.(new Error("spawn failed"));
		});

		expect(onSelectWorktree).not.toHaveBeenCalled();
		expect(await screen.findByRole("alert")).toHaveTextContent("spawn failed");
	});

	it("選択が起動中表示のまま作成が失敗した場合は同一launchTokenのエラー表示を再選択する", async () => {
		const user = userEvent.setup();
		const createCall = mockDeferredProviderCreate();
		const { onSelectWorktree, rerenderWorkspaceList } = renderWorkspaceList();
		wireSelectionRoundTrip({ onSelectWorktree, rerenderWorkspaceList });

		await launchProviderCreate(user, onSelectWorktree);
		const launching = onSelectWorktree.mock.lastCall?.[3] as Extract<
			CenterSelection,
			{ kind: "agent_session_launching" }
		>;

		await act(async () => {
			createCall.reject?.(new Error("spawn failed"));
		});

		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "agent_session_launching",
				worktreePath: "/repo/wt",
				provider: "codex",
				launchToken: launching.launchToken,
				error: "spawn failed",
			},
		);
	});

	it("closes only a Node allowed by backend capability", async () => {
		const user = userEvent.setup();
		mocks.invoke.mockResolvedValue(null);
		const detailRefresh = vi.fn();
		window.addEventListener("workspace-tree-refresh", detailRefresh);
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Close Direct session" }),
		);

		expect(mocks.invoke).toHaveBeenCalledWith("close_workspace_node", {
			worktreePath: "/repo/wt",
			nodeId: directNode.id,
		});
		expect(mocks.refreshTree).toHaveBeenCalledOnce();
		expect(detailRefresh).toHaveBeenCalledOnce();
		window.removeEventListener("workspace-tree-refresh", detailRefresh);
	});

	it("notifies App after Archive refresh says the current selection left the snapshot", async () => {
		const user = userEvent.setup();
		const selectedNodeId = "workflow-session-internal-uuid";
		const archivableTree = recursiveTree.map((item) =>
			item.kind === "workflow"
				? {
						...item,
						status: "completed" as const,
						capabilities: { ...item.capabilities, canArchive: true },
					}
				: item,
		);
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: archivableTree,
		});
		const onWorkspaceSelectionInvalidated = vi.fn();
		const { rerenderWorkspaceList } = renderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: selectedNodeId,
			},
			onWorkspaceSelectionInvalidated,
		});

		await user.click(
			screen.getByRole("button", { name: "Archive Release workflow" }),
		);
		await waitFor(() =>
			expect(mocks.beginArchiveReconciliation).toHaveBeenCalledWith(
				selectedNodeId,
			),
		);
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [directNode],
			preferredNodeId: directNode.id,
			reconciliationEvent: {
				refreshSeq: 2,
				requestContext: {
					worktreePath: "/repo/wt",
					selectedNodeId,
					reconciliationGeneration: 2,
				},
				selectionInSnapshot: false,
			},
		});
		rerenderWorkspaceList();

		await waitFor(() =>
			expect(onWorkspaceSelectionInvalidated).toHaveBeenCalledWith(
				"/repo/wt",
				selectedNodeId,
			),
		);
		expect(mocks.invoke).toHaveBeenCalledWith(
			"archive_workspace_workflow_execution",
			{
				worktreePath: "/repo/wt",
				executionId: "workflow-internal-uuid",
			},
		);
		rerenderWorkspaceList();
		expect(onWorkspaceSelectionInvalidated).toHaveBeenCalledOnce();
		expect(mocks.refreshTree).not.toHaveBeenCalled();
		expect(mocks.selectedNodeIds.get("/repo/wt")).toBe(selectedNodeId);
	});

	it("keeps the current selection when Archive reconciliation says it remains displayed", async () => {
		const user = userEvent.setup();
		const selectedNodeId = "workflow-session-internal-uuid";
		const archivableTree = recursiveTree.map((item) =>
			item.kind === "workflow"
				? {
						...item,
						status: "completed" as const,
						capabilities: { ...item.capabilities, canArchive: true },
					}
				: item,
		);
		mocks.treeStateOverrides.set("/repo/wt", { nodes: archivableTree });
		const onWorkspaceSelectionInvalidated = vi.fn();
		const { rerenderWorkspaceList } = renderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: selectedNodeId,
			},
			onWorkspaceSelectionInvalidated,
		});

		await user.click(
			screen.getByRole("button", { name: "Archive Release workflow" }),
		);
		await waitFor(() =>
			expect(mocks.beginArchiveReconciliation).toHaveBeenCalledWith(
				selectedNodeId,
			),
		);
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: archivableTree,
			preferredNodeId: selectedNodeId,
			reconciliationEvent: {
				refreshSeq: 2,
				requestContext: {
					worktreePath: "/repo/wt",
					selectedNodeId,
					reconciliationGeneration: 2,
				},
				selectionInSnapshot: true,
			},
		});
		rerenderWorkspaceList();

		expect(onWorkspaceSelectionInvalidated).not.toHaveBeenCalled();
	});

	it("does not deliver an accepted invalidation after the selection moves", async () => {
		const selectedNodeId = "workflow-session-internal-uuid";
		const onWorkspaceSelectionInvalidated = vi.fn();
		const { rerenderWorkspaceList } = renderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: selectedNodeId,
			},
			onWorkspaceSelectionInvalidated,
		});
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [directNode],
			preferredNodeId: directNode.id,
			reconciliationEvent: {
				refreshSeq: 4,
				requestContext: {
					worktreePath: "/repo/wt",
					selectedNodeId,
					reconciliationGeneration: 3,
				},
				selectionInSnapshot: false,
			},
		});

		rerenderWorkspaceList({
			centerSelection: {
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: directNode.id,
			},
		});

		expect(onWorkspaceSelectionInvalidated).not.toHaveBeenCalled();
		expect(mocks.selectedNodeIds.get("/repo/wt")).toBe(directNode.id);
	});

	it("resumes a Provider history candidate as a new AgentSession", async () => {
		const user = userEvent.setup();
		mocks.invoke.mockImplementation((command) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({ nodes: [], sessions: [] });
			}
			if (command === "list_agent_session_history") {
				return Promise.resolve({
					items: [
						{
							provider: "codex",
							providerSessionId: "provider-session-1",
						},
					],
					nextAfter: null,
				});
			}
			if (command === "resume_agent_session_history_candidate") {
				return Promise.resolve("agent-session-2");
			}
			return Promise.resolve(null);
		});
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.hover(screen.getByRole("menuitem", { name: "SessionHistory" }));
		const candidate = await screen.findByRole("menuitem", {
			name: /codex.*provider-session-1/,
		});
		act(() => candidate.focus());
		await user.keyboard("{Enter}");

		expect(mocks.invoke).toHaveBeenCalledWith(
			"resume_agent_session_history_candidate",
			expect.objectContaining({
				workspaceIdentity: "/repo/wt",
				worktreePath: "/repo/wt",
				provider: "codex",
				providerSessionId: "provider-session-1",
				rows: 24,
				cols: 80,
				callerRequestId: expect.any(String),
			}),
		);
		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "agent_session",
				worktreePath: "/repo/wt",
				agentSessionId: "agent-session-2",
			},
		);
	});

	it("Provider historyの次pageをcursorから表示する", async () => {
		const user = userEvent.setup();
		mocks.invoke.mockImplementation((command, args?: unknown) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({ nodes: [], sessions: [] });
			}
			if (command === "list_agent_session_history") {
				const after = (args as { after?: string })?.after;
				return Promise.resolve(
					after
						? {
								items: [
									{
										provider: "claude",
										providerSessionId: "provider-session-2",
									},
								],
								nextAfter: null,
							}
						: {
								items: [
									{
										provider: "codex",
										providerSessionId: "provider-session-1",
									},
								],
								nextAfter: "history-cursor-1",
							},
				);
			}
			return Promise.resolve(null);
		});
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.hover(screen.getByRole("menuitem", { name: "SessionHistory" }));
		const loadMore = await screen.findByRole("menuitem", {
			name: "Load more Provider history",
		});
		act(() => loadMore.focus());
		await user.keyboard("{Enter}");
		await waitFor(() =>
			expect(mocks.invoke).toHaveBeenCalledWith("list_agent_session_history", {
				worktreePath: "/repo/wt",
				limit: 100,
				after: "history-cursor-1",
			}),
		);
		expect(
			await screen.findByRole("menuitem", {
				name: /claude.*provider-session-2/,
			}),
		).toBeVisible();
	});

	it("invalidates Node detail when Close commits but tree refresh fails", async () => {
		const user = userEvent.setup();
		mocks.invoke.mockResolvedValue(null);
		mocks.refreshTree.mockRejectedValueOnce(new Error("tree offline"));
		const detailRefresh = vi.fn();
		window.addEventListener("workspace-tree-refresh", detailRefresh);
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Close Direct session" }),
		);

		await waitFor(() => expect(detailRefresh).toHaveBeenCalledOnce());
		expect(mocks.invoke).toHaveBeenCalledWith("close_workspace_node", {
			worktreePath: "/repo/wt",
			nodeId: directNode.id,
		});
		expect(mocks.refreshTree).toHaveBeenCalledOnce();
		expect(screen.getByRole("alert")).toHaveTextContent("tree offline");
		window.removeEventListener("workspace-tree-refresh", detailRefresh);
	});

	it("enables Workflow actions only from backend capabilities", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for Release workflow" }),
		);
		expect(screen.getByRole("menuitem", { name: "Stop" })).toBeEnabled();
		expect(screen.getByRole("menuitem", { name: "Resume" })).toHaveAttribute(
			"aria-disabled",
			"true",
		);
		expect(screen.getByRole("menuitem", { name: "Abort" })).toBeEnabled();
	});
});
