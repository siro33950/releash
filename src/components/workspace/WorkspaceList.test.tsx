import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceTreeReconciliationEvent } from "@/hooks/useWorkspaceTreeNodes";
import type { AgentSessionItem } from "@/types/agent-session";
import type { WorktreeBranch } from "@/types/git";
import type {
	CenterSelection,
	WorkspaceNode,
	WorkspaceTreeItem,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { WorkspaceList } from "./WorkspaceList";

type MockWorkspaceTreeState = {
	nodes: WorkspaceTreeItem[];
	archivedSessions?: AgentSessionItem[];
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
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: mocks.invoke,
	listenClient: mocks.listen,
}));
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
		const [archivedSessions, setArchivedSessions] = useState<
			AgentSessionItem[]
		>(state.archivedSessions ?? []);
		useEffect(() => {
			if (state.archivedSessions) {
				setArchivedSessions(state.archivedSessions);
				return;
			}
			let active = true;
			void mocks
				.invoke("list_workspace_worktree_nodes", { worktreePath })
				.then((snapshot: unknown) => {
					if (!active) return;
					setArchivedSessions(
						(snapshot as { archivedSessions?: AgentSessionItem[] } | null)
							?.archivedSessions ?? [],
					);
				});
			return () => {
				active = false;
			};
		}, [state.archivedSessions, worktreePath]);
		return {
			nodes: state.nodes,
			archivedSessions,
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
		loading: false,
		refresh: mocks.refreshWorktrees,
	}),
}));

const directNode: WorkspaceTreeItem = {
	kind: "node",
	processPresence: "unknown",
	id: "4f168b74-f9cf-4d51-9970-81ea281bc983",
	title: "Direct session",
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
};

function standaloneSessionNode({
	id,
	title,
	status = "active",
	canArchive = true,
	canDelete = false,
	canRename = false,
	sessionRef = id,
}: {
	id: string;
	title: string;
	status?: WorkspaceNode["status"];
	canArchive?: boolean;
	canDelete?: boolean;
	canRename?: boolean;
	sessionRef?: string;
}): WorkspaceNode {
	return {
		kind: "node",
		processPresence: "unknown",
		id,
		title,
		status,
		contentKind: "session",
		capabilities: {
			canRename,
			canApprove: false,
			canRetry: false,
			canResumeSession: false,
		},
		sessionCapabilities: {
			sessionRef,
			canArchive,
			canDelete,
		},
		pastAttempts: [],
		pastAttemptsCollapsed: false,
		updatedAt: 1,
	};
}

const recursiveTree: WorkspaceTreeItem[] = [
	directNode,
	{
		kind: "sequence",
		id: "workflow-internal-uuid",
		title: "Release workflow",
		status: "active",
		workflowCapabilities: {
			canAbort: true,
			canArchive: false,
		},
		updatedAt: 2,
		children: [
			{
				kind: "node",
				processPresence: "unknown",
				id: "workflow-session-internal-uuid",
				title: "Prepare",
				status: "idle",
				contentKind: "session",
				capabilities: {
					canRename: false,
					canApprove: false,
					canRetry: false,
					canResumeSession: false,
				},
				pastAttempts: [],
				pastAttemptsCollapsed: false,
				updatedAt: 3,
			},
			{
				kind: "fanout",
				id: "fanout-internal-uuid",
				title: "Review all",
				status: "active",
				workflowCapabilities: null,
				updatedAt: 4,
				children: [
					{
						kind: "sequence",
						id: "review-sequence-internal-uuid",
						title: "Review sequence",
						status: "attention",
						updatedAt: 5,
						children: [
							{
								kind: "node",
								processPresence: "unknown",
								id: "fanout-child-internal-uuid",
								title: "Architecture review",
								status: "active",
								contentKind: "command",
								capabilities: {
									canRename: false,
									canApprove: false,
									canRetry: false,
									canResumeSession: false,
								},
								pastAttempts: [],
								pastAttemptsCollapsed: false,
								updatedAt: 6,
							},
						],
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
		reject?: (error: unknown) => void;
	} = {};
	mocks.invoke.mockImplementation((command: string) => {
		if (command === "list_workspace_worktree_nodes") {
			return Promise.resolve({ nodes: [], archivedSessions: [] });
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
		if (command === "get_workspace_session_node_id") {
			return Promise.resolve("agent-session-node-1");
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
	vi.mocked(invokeTauri).mockReset().mockResolvedValue(null);
	for (const mock of Object.values(mocks)) {
		if (typeof mock === "function" && "mockClear" in mock) {
			mock.mockClear();
		}
	}
	mocks.worktreeBranches = [makeBranch()];
	mocks.treeStateOverrides.clear();
	mocks.selectedNodeIds.clear();
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
	it("通常のworktree一覧に掃除候補sectionを表示しない", () => {
		mocks.worktreeBranches = [makeBranch()];
		renderWorkspaceList();
		expect(screen.queryByLabelText("掃除候補")).not.toBeInTheDocument();
	});

	it("renders Waypoints for top-level and nested Sequence rows while keeping GitFork for Fanout", () => {
		const { container } = renderWorkspaceList();

		expect(screen.getByText("Release workflow")).toBeInTheDocument();
		expect(screen.getByText("Review all")).toBeInTheDocument();
		expect(screen.getByText("Review sequence")).toBeInTheDocument();
		expect(screen.getByText("Architecture review")).toBeInTheDocument();
		expect(
			screen
				.getByRole("button", { name: "Release workflow" })
				.querySelector("svg.lucide-waypoints"),
		).toBeInTheDocument();
		expect(
			screen
				.getByRole("button", { name: "Review sequence" })
				.querySelector("svg.lucide-waypoints"),
		).toBeInTheDocument();
		expect(
			screen
				.getByRole("button", { name: "Review all" })
				.querySelector("svg.lucide-git-fork"),
		).toBeInTheDocument();
		expect(container.querySelectorAll("svg.lucide-waypoints")).toHaveLength(2);
		expect(container.querySelectorAll("svg.lucide-list-tree")).toHaveLength(0);
		expect(container.querySelectorAll("svg.lucide-git-fork")).toHaveLength(1);
	});

	it("uses the four classification colors and pulse rules for Sequence rows", () => {
		const cases = [
			{
				title: "Active sequence",
				status: "active",
				colorClasses: ["text-blue-600", "dark:text-blue-300"],
				pulses: true,
			},
			{
				title: "Attention sequence",
				status: "attention",
				colorClasses: ["text-yellow-600", "dark:text-yellow-300"],
				pulses: true,
			},
			{
				title: "Failure sequence",
				status: "failure",
				colorClasses: ["text-red-600", "dark:text-red-300"],
				pulses: false,
			},
			{
				title: "Idle sequence",
				status: "idle",
				colorClasses: ["text-green-600", "dark:text-green-300"],
				pulses: false,
			},
		] as const;
		const nodes: WorkspaceTreeItem[] = cases.map(
			({ title, status }, index) => ({
				kind: "sequence",
				id: `sequence-${index}`,
				title,
				status,
				children: [],
				updatedAt: index,
			}),
		);
		mocks.treeStateOverrides.set("/repo/wt", { nodes });

		const { container } = renderWorkspaceList();

		for (const { title, colorClasses, pulses } of cases) {
			const icon = screen
				.getByRole("button", { name: title })
				.querySelector("svg.lucide-waypoints");
			expect(icon).toBeInTheDocument();
			expect(icon).toHaveClass("size-3.5");
			expect(icon).toHaveClass(...colorClasses);
			if (pulses) {
				expect(icon).toHaveClass("animate-pulse");
			} else {
				expect(icon).not.toHaveClass("animate-pulse");
			}
		}
		expect(container.querySelectorAll("svg.lucide-list-tree")).toHaveLength(0);
	});

	it("backendが絞り込んだStandalone Session Nodeを選択できる", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "provider-agent-node-1",
					title: "Claude AgentSession",
				}),
			],
			archivedSessions: [],
		});
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(
			screen.getByRole("button", {
				name: "Claude AgentSession, active",
			}),
		);

		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: "provider-agent-node-1",
			},
		);
	});

	it("Archived AgentSessionをWorkspaceからSessionHistoryへ移して復帰できる", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [],
			archivedSessions: [
				{
					id: "provider-agent-archived",
					workspaceIdentity: "/repo/wt",
					workspaceWorktreePath: "/repo/wt",
					worktreePath: "/repo-worktrees/.releash-isolated/restored-a1",
					provider: "claude",
					treeLocation: {
						treeId: "provider-agent-archived",
						nodeExecutionId: "provider-agent-archived",
					},
					lifecycle: "archived",
					lastExitAbnormal: false,
					operations: {
						canArchive: false,
						canRestore: true,
						canDelete: true,
						canResume: false,
					},
				},
			],
		});
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "list_agent_session_history") {
				return Promise.resolve({ items: [], nextAfter: null });
			}
			if (command === "restore_agent_session") {
				return Promise.resolve("restored");
			}
			if (command === "get_workspace_session_node_id") {
				return Promise.resolve("restored-session-node");
			}
			return Promise.resolve(null);
		});
		const user = userEvent.setup();
		const changed = vi.fn();
		window.addEventListener("agent-session-refresh", changed);
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
					kind: "node",
					worktreePath: "/repo/wt",
					nodeId: "restored-session-node",
					initialSessionAttachment: {
						agentSessionId: "provider-agent-archived",
						workspaceIdentity: "/repo/wt",
						worktreePath: "/repo-worktrees/.releash-isolated/restored-a1",
						workspaceWorktreePath: "/repo/wt",
						provider: "claude",
					},
				},
			);
		});
		expect(changed).toHaveBeenCalledWith(
			expect.objectContaining({ detail: { worktreePath: "/repo/wt" } }),
		);
		window.removeEventListener("agent-session-refresh", changed);
	});

	it("Standalone Session Nodeの4分類は色とpulseで表現する", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "provider-agent-active",
					title: "Active Session",
					status: "active",
				}),
				standaloneSessionNode({
					id: "provider-agent-attention",
					title: "Attention Session",
					status: "attention",
				}),
				standaloneSessionNode({
					id: "provider-agent-failure",
					title: "Failure Session",
					status: "failure",
				}),
				standaloneSessionNode({
					id: "provider-agent-idle",
					title: "Idle Session",
					status: "idle",
				}),
			],
			archivedSessions: [],
		});
		renderWorkspaceList();

		const activeRow = screen.getByRole("button", {
			name: "Active Session, active",
		});
		const attentionRow = screen.getByRole("button", {
			name: "Attention Session, attention",
		});
		const failureRow = screen.getByRole("button", {
			name: "Failure Session, failure",
		});
		const idleRow = screen.getByRole("button", {
			name: "Idle Session, idle",
		});

		expect(within(activeRow).queryByText("active")).toBeNull();
		expect(within(attentionRow).queryByText("attention")).toBeNull();
		expect(within(failureRow).queryByText("failure")).toBeNull();
		expect(within(idleRow).queryByText("idle")).toBeNull();
		expect(
			within(activeRow).getByTitle("session, active").firstChild,
		).toHaveClass("text-blue-600", "dark:text-blue-300", "animate-pulse");
		expect(
			within(attentionRow).getByTitle("session, attention").firstChild,
		).toHaveClass("text-yellow-600", "dark:text-yellow-300", "animate-pulse");
		expect(
			within(failureRow).getByTitle("session, failure").firstChild,
		).toHaveClass("text-red-600", "dark:text-red-300");
		expect(within(idleRow).getByTitle("session, idle").firstChild).toHaveClass(
			"text-green-600",
			"dark:text-green-300",
		);
		expect(
			within(failureRow).getByTitle("session, failure").firstChild,
		).not.toHaveClass("animate-pulse");
		expect(
			within(idleRow).getByTitle("session, idle").firstChild,
		).not.toHaveClass("animate-pulse");
	});

	it("bind前のSession Nodeを灰色の回転loaderで描画しrename入口を出さない", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "unbound-session",
					title: "session",
					status: "unbound",
					canRename: false,
				}),
			],
			archivedSessions: [],
		});
		renderWorkspaceList();

		const row = screen.getByRole("button", { name: "session, unbound" });
		const icon = within(row).getByTitle("session, unbound").firstChild;
		expect(icon).toHaveClass(
			"lucide-loader-circle",
			"animate-spin",
			"text-muted-foreground",
		);
		expect(
			screen.queryByRole("button", { name: "Rename session" }),
		).not.toBeInTheDocument();
	});

	it("canRenameが真のSession Nodeだけにrename入口を出す", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "renameable-session",
					title: "Renameable session",
					canRename: true,
				}),
				standaloneSessionNode({
					id: "fixed-session",
					title: "Fixed session",
					canRename: false,
				}),
			],
			archivedSessions: [],
		});

		renderWorkspaceList();

		expect(
			screen.getByRole("button", { name: "Rename Renameable session" }),
		).toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Rename Fixed session" }),
		).not.toBeInTheDocument();
	});

	it("rename入力は現在名を全選択しEnterでbackend commandだけを呼ぶ", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "renameable-session",
					title: "Current title",
					canRename: true,
				}),
			],
			archivedSessions: [],
		});
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Rename Current title" }),
		);
		const input = screen.getByRole("textbox", {
			name: "Rename Current title",
		}) as HTMLInputElement;
		expect(input).toHaveValue("Current title");
		expect(input.selectionStart).toBe(0);
		expect(input.selectionEnd).toBe("Current title".length);
		await user.clear(input);
		await user.type(input, "  New title  {Enter}");

		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"rename_workspace_session_node",
				{
					worktreePath: "/repo/wt",
					nodeId: "renameable-session",
					name: "  New title  ",
				},
			);
		});
		expect(screen.getByText("Current title")).toBeInTheDocument();
		expect(screen.queryByText("New title")).not.toBeInTheDocument();
	});

	it("rename失敗時は入力を保持し再送信の成功後に編集を閉じる", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "retry-rename-session",
					title: "Current title",
					canRename: true,
				}),
			],
			archivedSessions: [],
		});
		const rename = vi
			.fn()
			.mockRejectedValueOnce(new Error("Rename failed"))
			.mockResolvedValueOnce(undefined);
		mocks.invoke.mockImplementation((command: string, args: unknown) => {
			if (command === "rename_workspace_session_node") return rename(args);
			return Promise.resolve(null);
		});
		const user = userEvent.setup();
		renderWorkspaceList();
		await user.click(
			screen.getByRole("button", { name: "Rename Current title" }),
		);
		const input = screen.getByRole("textbox", {
			name: "Rename Current title",
		});
		await user.clear(input);
		await user.type(input, "New title{Enter}");

		expect(await screen.findByText("Rename failed")).toBeVisible();
		expect(input).toHaveValue("New title");
		expect(input).toHaveFocus();

		await user.keyboard("{Enter}");

		await waitFor(() => {
			expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
		});
		expect(rename).toHaveBeenCalledTimes(2);
		expect(rename).toHaveBeenNthCalledWith(2, {
			worktreePath: "/repo/wt",
			nodeId: "retry-rename-session",
			name: "New title",
		});
		expect(screen.queryByText("Rename failed")).not.toBeInTheDocument();
	});

	it.each([
		["Escape", "{Escape}"],
		["blur", "{Tab}"],
		["whitespace", "   {Enter}"],
	])("rename入力は%sで取り消しinvokeしない", async (_caseName, keys) => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "cancel-rename-session",
					title: "Stable title",
					canRename: true,
				}),
			],
			archivedSessions: [],
		});
		const user = userEvent.setup();
		renderWorkspaceList();
		await user.click(
			screen.getByRole("button", { name: "Rename Stable title" }),
		);
		const input = screen.getByRole("textbox", {
			name: "Rename Stable title",
		});
		if (_caseName === "whitespace") {
			await user.clear(input);
		}

		await user.keyboard(keys);

		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"rename_workspace_session_node",
			expect.anything(),
		);
		expect(
			screen.queryByRole("textbox", { name: "Rename Stable title" }),
		).not.toBeInTheDocument();
	});

	it("Standalone AgentSessionのXはArchiveしID不明時はDelete確認を要求する", async () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "provider-agent-unknown",
					title: "Claude AgentSession",
				}),
			],
			archivedSessions: [],
		});
		mocks.invoke.mockImplementation((command: string) => {
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
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "provider-agent-known",
					title: "Claude AgentSession",
				}),
			],
			archivedSessions: [],
		});
		mocks.invoke.mockImplementation((command: string) => {
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

	it("Standalone Session Node一覧を共通snapshotから表示する", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "provider-agent-2",
					title: "Codex AgentSession",
				}),
				standaloneSessionNode({
					id: "provider-agent-1",
					title: "Claude AgentSession",
				}),
			],
			archivedSessions: [],
		});
		renderWorkspaceList();

		expect(
			screen.getByRole("button", {
				name: "Codex AgentSession, active",
			}),
		).toBeVisible();
		expect(screen.getByText("Claude AgentSession")).toBeVisible();
	});

	it("renders an empty backend-owned Sequence branch without Node leaves", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				{
					kind: "sequence",
					id: "empty-workflow",
					title: "Empty workflow",
					status: "active",
					workflowCapabilities: {
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

	it("keeps one familiar content icon per Node and styles it from backend classification", () => {
		renderWorkspaceList();

		const sessionRow = screen.getByRole("button", {
			name: "Direct session, active",
		});
		const sessionIcons = sessionRow.querySelectorAll("svg");
		expect(sessionIcons).toHaveLength(1);
		expect(sessionIcons[0]).toHaveClass(
			"lucide-bot",
			"text-blue-600",
			"animate-pulse",
		);

		const commandRow = screen.getByRole("button", {
			name: "Architecture review, active",
		});
		const commandIcons = commandRow.querySelectorAll("svg");
		expect(commandIcons).toHaveLength(1);
		expect(commandIcons[0]).toHaveClass(
			"lucide-terminal",
			"text-blue-600",
			"animate-pulse",
		);
	});

	it("shows the backend failure classification on a failed session badge", () => {
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				{
					...directNode,
					status: "failure",
					errorReason: "app server stopped",
				},
			],
		});

		renderWorkspaceList();

		expect(screen.getByTitle("session, failure")).toBeInTheDocument();
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
			processPresence: "unknown",
			id: "occurrence-a-1",
			title: "A",
			status: "idle",
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
			status: "active",
			updatedAt: 4,
		};
		const workflow = (children: WorkspaceTreeItem[]): WorkspaceTreeItem => ({
			kind: "sequence",
			id: "loop-workflow",
			title: "Loop workflow",
			status: "active",
			workflowCapabilities: {
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
			"A, idle",
			"B, idle",
			"A, idle",
			"C, active",
		]);
		const [firstA, secondA] = screen.getAllByRole("button", {
			name: /^A, idle$/,
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

	it("retryの決着済み過去実行を既定で折り畳み、実行順に展開して選択できる", async () => {
		const user = userEvent.setup();
		const first = standaloneSessionNode({
			id: "retry-attempt-first",
			title: "Review",
			status: "failure",
			canArchive: false,
		});
		const second = standaloneSessionNode({
			id: "retry-attempt-second",
			title: "Review",
			status: "idle",
			canArchive: false,
		});
		const latest: WorkspaceNode = {
			...standaloneSessionNode({
				id: "retry-attempt-latest",
				title: "Review",
				status: "active",
				canArchive: false,
			}),
			pastAttempts: [first, second],
			pastAttemptsCollapsed: true,
		};
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [latest],
			archivedSessions: [],
		});
		const { onSelectWorktree } = renderWorkspaceList();

		expect(screen.getAllByRole("button", { name: /^Review,/ })).toHaveLength(1);
		expect(screen.queryByText(/Attempt \d+/)).not.toBeInTheDocument();
		await user.click(
			screen.getByRole("button", {
				name: "Show past executions for Review",
			}),
		);

		const executions = screen.getAllByRole("button", { name: /^Review,/ });
		expect(executions.map((row) => row.getAttribute("aria-label"))).toEqual([
			"Review, failure",
			"Review, idle",
			"Review, active",
		]);
		await user.click(executions[0]);
		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: "retry-attempt-first",
			},
		);
	});

	it("過去attemptの履歴展開で当該attemptのdelegateの子と部分木を表示して選択できる", async () => {
		const user = userEvent.setup();
		const grandchild = standaloneSessionNode({
			id: "past-delegate-grandchild",
			title: "Past judge",
			status: "idle",
			canArchive: false,
		});
		const child: WorkspaceNode = {
			...standaloneSessionNode({
				id: "past-delegate-child",
				title: "Past verify",
				status: "idle",
				canArchive: false,
			}),
			children: [grandchild],
		};
		const past: WorkspaceNode = {
			...standaloneSessionNode({
				id: "past-delegate-parent",
				title: "Implement",
				status: "failure",
				canArchive: false,
			}),
			children: [child],
		};
		const latest: WorkspaceNode = {
			...standaloneSessionNode({
				id: "latest-delegate-parent",
				title: "Implement",
				status: "active",
				canArchive: false,
			}),
			pastAttempts: [past],
			pastAttemptsCollapsed: true,
			children: [
				standaloneSessionNode({
					id: "latest-delegate-child",
					title: "Current verify",
					status: "active",
					canArchive: false,
				}),
			],
		};
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [latest],
			archivedSessions: [],
		});
		const { onSelectWorktree } = renderWorkspaceList();

		expect(screen.queryByRole("button", { name: /^Past / })).toBeNull();
		await user.click(
			screen.getByRole("button", {
				name: "Show past executions for Implement",
			}),
		);

		const rows = screen.getAllByRole("button", {
			name: /^(Implement|Past verify|Past judge|Current verify),/,
		});
		expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual([
			"Implement, failure",
			"Past verify, idle",
			"Past judge, idle",
			"Implement, active",
			"Current verify, active",
		]);
		for (const [row, nodeId] of [
			[rows[1], child.id],
			[rows[2], grandchild.id],
		] as const) {
			await user.click(row);
			expect(onSelectWorktree).toHaveBeenLastCalledWith(
				"/repo/wt",
				"feature",
				"repo",
				{ kind: "node", worktreePath: "/repo/wt", nodeId },
			);
		}
		await user.click(
			screen.getByRole("button", {
				name: "Hide past executions for Implement",
			}),
		);
		expect(screen.queryByRole("button", { name: /^Past / })).toBeNull();
		expect(
			screen.getByRole("button", { name: "Current verify, active" }),
		).toBeInTheDocument();
	});

	it("delegateの子を親Sessionの下に発火順で表示して選択できる", async () => {
		const user = userEvent.setup();
		const first = standaloneSessionNode({
			id: "delegate-first",
			title: "Verify 1",
			status: "idle",
			canArchive: false,
		});
		const second = standaloneSessionNode({
			id: "delegate-second",
			title: "Verify 2",
			status: "active",
			canArchive: false,
		});
		const parent: WorkspaceNode = {
			...standaloneSessionNode({
				id: "delegate-parent",
				title: "Implement",
				status: "active",
				canArchive: false,
			}),
			children: [first, second],
		};
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [parent],
			archivedSessions: [],
		});
		const { onSelectWorktree } = renderWorkspaceList();
		const rows = screen.getAllByRole("button", {
			name: /^(Implement|Verify [12]),/,
		});
		expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual([
			"Implement, active",
			"Verify 1, idle",
			"Verify 2, active",
		]);
		await user.click(rows[1]);
		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{ kind: "node", worktreePath: "/repo/wt", nodeId: "delegate-first" },
		);
	});

	it("NewSessionはNewWorkflowと同じsubmenuでProviderを選択して作成する", async () => {
		const user = userEvent.setup();
		let providerSessionListCalls = 0;
		const refreshAfterCreate = new Promise(() => {});
		mocks.invoke.mockImplementation((command) => {
			if (command === "list_workspace_worktree_nodes") {
				providerSessionListCalls += 1;
				return providerSessionListCalls === 1
					? Promise.resolve({ nodes: [], archivedSessions: [] })
					: refreshAfterCreate;
			}
			if (command === "list_available_agent_session_providers") {
				return Promise.resolve(["codex"]);
			}
			if (command === "create_agent_session") {
				return Promise.resolve("agent-session-1");
			}
			if (command === "get_workspace_session_node_id") {
				return Promise.resolve("agent-session-node-1");
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
					kind: "node",
					worktreePath: "/repo/wt",
					nodeId: "agent-session-node-1",
					initialSessionAttachment: {
						agentSessionId: "agent-session-1",
						workspaceIdentity: "/repo/wt",
						worktreePath: "/repo/wt",
						workspaceWorktreePath: "/repo/wt",
						provider: "codex",
					},
				},
			);
		});
	});

	it("AgentSession作成は通信状態を表示せず同じ起動選択へ成功を反映する", async () => {
		const user = userEvent.setup();
		const createCall = mockDeferredProviderCreate();
		const { onSelectWorktree, rerenderWorkspaceList } = renderWorkspaceList();
		wireSelectionRoundTrip({ onSelectWorktree, rerenderWorkspaceList });
		await launchProviderCreate(user, onSelectWorktree);
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		await act(async () => createCall.resolve?.("agent-session-1"));
		expect(onSelectWorktree).toHaveBeenLastCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			expect.objectContaining({
				kind: "node",
				nodeId: "agent-session-node-1",
				initialSessionAttachment: expect.objectContaining({
					agentSessionId: "agent-session-1",
				}),
			}),
		);
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) => command === "create_agent_session",
			),
		).toHaveLength(1);
		expect(
			screen.queryByText(/操作結果を確認できません/),
		).not.toBeInTheDocument();
	});

	it("AgentSession作成のpending中に別Nodeへ移動した場合は選択を奪わない", async () => {
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

	it.each(["success", "error"])(
		"workflow開始は通信状態を表示せず応答の%sを反映する",
		async (outcome) => {
			const user = userEvent.setup();
			let complete!: (value: string) => void;
			let reject!: (error: unknown) => void;
			let onUncertain: unknown;
			mocks.invoke.mockImplementation(
				(command: string, _args: unknown, options: unknown) => {
					if (command !== "start_workflow") return Promise.resolve(null);
					onUncertain = options;
					return new Promise<string>((resolve, fail) => {
						complete = resolve;
						reject = fail;
					});
				},
			);
			renderWorkspaceList();
			await user.click(
				screen.getByRole("button", { name: "Create in feature" }),
			);
			await user.hover(screen.getByRole("menuitem", { name: "NewWorkflow" }));
			const workflow = await screen.findByRole("menuitem", { name: /release/ });
			act(() => workflow.focus());
			await user.keyboard("{Enter}");
			await user.type(
				screen.getByRole("textbox", { name: "Workflow request" }),
				"release request",
			);
			await user.click(screen.getByRole("button", { name: "Start" }));
			expect(
				screen.getByRole("button", { name: "Starting..." }),
			).toBeDisabled();
			expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
			expect(
				screen.getByRole("textbox", { name: "Workflow request" }),
			).toBeDisabled();
			expect(onUncertain).toBeUndefined();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(
				mocks.invoke.mock.calls.filter(
					([command]) => command === "start_workflow",
				),
			).toEqual([
				[
					"start_workflow",
					{
						workflowName: "release",
						worktreePath: "/repo/wt",
						request: "release request",
					},
				],
			]);
			mocks.refreshTree.mockClear();
			await act(async () => {
				if (outcome === "success") complete("workflow-1");
				else reject(new Error("workflow start failed"));
			});
			if (outcome === "success") {
				expect(
					screen.queryByRole("dialog", { name: "NewWorkflow" }),
				).not.toBeInTheDocument();
				expect(mocks.refreshTree).toHaveBeenCalledTimes(1);
			} else {
				expect(screen.getByRole("alert")).toHaveTextContent(
					"workflow start failed",
				);
				expect(
					screen.getByRole("textbox", { name: "Workflow request" }),
				).toHaveValue("release request");
			}
			expect(
				screen.queryByText(/操作結果を確認できません/),
			).not.toBeInTheDocument();
		},
	);

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
			createCall.reject?.({
				code: "AGENT_SESSION_LAUNCH_UNAVAILABLE",
				message: "backend launch failed",
			});
		});

		expect(onSelectWorktree).not.toHaveBeenCalled();
		expect((await screen.findByRole("alert")).textContent).toBe(
			"backend launch failed",
		);
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
			createCall.reject?.("plain launch failed");
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
				error: "plain launch failed",
			},
		);
	});

	it("未登録のClose操作を表示せずTauri commandを呼ばない", () => {
		renderWorkspaceList();
		expect(
			screen.queryByRole("button", { name: "Close Direct session" }),
		).not.toBeInTheDocument();
		expect(invokeTauri).not.toHaveBeenCalled();
	});

	it("notifies App after Archive refresh says the current selection left the snapshot", async () => {
		const user = userEvent.setup();
		const selectedNodeId = "workflow-session-internal-uuid";
		const archivableTree = recursiveTree.map((item) =>
			item.kind === "sequence" && item.workflowCapabilities
				? {
						...item,
						status: "idle" as const,
						workflowCapabilities: {
							...item.workflowCapabilities,
							canArchive: true,
						},
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
			item.kind === "sequence" && item.workflowCapabilities
				? {
						...item,
						status: "idle" as const,
						workflowCapabilities: {
							...item.workflowCapabilities,
							canArchive: true,
						},
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
				return Promise.resolve({ nodes: [], archivedSessions: [] });
			}
			if (command === "list_agent_session_history") {
				return Promise.resolve({
					items: [
						{
							provider: "codex",
							providerSessionId: "provider-session-1",
							label: "Fix provider history labels",
						},
					],
					nextAfter: null,
				});
			}
			if (command === "resume_agent_session_history_candidate") {
				return Promise.resolve("agent-session-2");
			}
			if (command === "get_workspace_session_node_id") {
				return Promise.resolve("agent-session-node-2");
			}
			return Promise.resolve(null);
		});
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.hover(screen.getByRole("menuitem", { name: "SessionHistory" }));
		const candidate = await screen.findByRole("menuitem", {
			name: "Fix provider history labels",
		});
		expect(screen.queryByText("provider-session-1")).not.toBeInTheDocument();
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
				kind: "node",
				worktreePath: "/repo/wt",
				nodeId: "agent-session-node-2",
				initialSessionAttachment: {
					agentSessionId: "agent-session-2",
					workspaceIdentity: "/repo/wt",
					worktreePath: "/repo/wt",
					workspaceWorktreePath: "/repo/wt",
					provider: "codex",
				},
			},
		);
	});

	it("Provider historyの次pageをcursorから表示する", async () => {
		const user = userEvent.setup();
		mocks.invoke.mockImplementation((command, args?: unknown) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve({ nodes: [], archivedSessions: [] });
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
										label: "Second provider conversation",
									},
								],
								nextAfter: null,
							}
						: {
								items: [
									{
										provider: "codex",
										providerSessionId: "provider-session-1",
										label: "First provider conversation",
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
				name: "Second provider conversation",
			}),
		).toBeVisible();
	});

	it("enables Workflow actions only from backend capabilities", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for Release workflow" }),
		);
		expect(
			screen.queryByRole("menuitem", { name: "Stop" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("menuitem", { name: "Resume" }),
		).not.toBeInTheDocument();
		expect(screen.getByRole("menuitem", { name: "Abort" })).toBeEnabled();
	});

	it("leaf Node rootからworkflow全体の操作を実行できる", async () => {
		const user = userEvent.setup();
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				{
					...standaloneSessionNode({
						id: "leaf-workflow-execution",
						title: "Leaf workflow",
						canArchive: false,
					}),
					sessionCapabilities: null,
					workflowCapabilities: {
						canAbort: true,
						canArchive: false,
					},
				},
			],
			archivedSessions: [],
		});
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for Leaf workflow" }),
		);
		await user.click(screen.getByRole("menuitem", { name: "Abort" }));

		expect(mocks.invoke).toHaveBeenCalledWith("abort_workflow", {
			executionId: "leaf-workflow-execution",
		});
	});

	it("Standalone Session NodeをopaqueなSession参照で削除できる", async () => {
		const user = userEvent.setup();
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				standaloneSessionNode({
					id: "standalone-node-id",
					title: "Deletable Session",
					canArchive: false,
					canDelete: true,
					sessionRef: "opaque-session-ref",
				}),
			],
			archivedSessions: [],
		});
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Delete Deletable Session" }),
		);

		expect(mocks.invoke).toHaveBeenCalledWith(
			"delete_agent_session",
			expect.objectContaining({
				agentSessionId: "opaque-session-ref",
				callerRequestId: expect.any(String),
			}),
		);
	});
});

it.each(["sequence", "fanout"] as const)(
	"隔離%sはchildrenが空でもbranchとpathを表示し展開を切り替えられる",
	async (kind) => {
		const worktree = {
			branch: "releash/isolated/composite-a2",
			path: "/repo-worktrees/.releash-isolated/composite-a2",
		};
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: [
				{
					kind,
					id: "composite",
					title: "Isolated composite",
					status: "idle",
					children: [],
					worktree,
					updatedAt: 1,
				},
			],
		});
		const user = userEvent.setup();
		renderWorkspaceList();
		expect(screen.getByText(worktree.branch)).toBeVisible();
		expect(screen.getByText(worktree.path)).toBeVisible();
		await user.click(
			screen.getByRole("button", { name: "Isolated composite" }),
		);
		expect(screen.queryByText(worktree.path)).toBeNull();
		await user.click(
			screen.getByRole("button", { name: "Isolated composite" }),
		);
		expect(screen.getByText(worktree.path)).toBeVisible();
	},
);

it.each([
	["remove_worktree", "success"],
	["remove_worktree", "failure"],
	["delete_branch", "success"],
	["delete_branch", "failure"],
] as const)(
	"%sは通信状態を表示せず応答の%sを反映する",
	async (command, outcome) => {
		const branch = {
			...makeBranch(),
			is_merged: true,
			worktree_path: command === "remove_worktree" ? "/repo/wt" : null,
		};
		mocks.worktreeBranches = [branch];
		let options: unknown;
		let complete!: () => void;
		let fail!: (error: Error) => void;
		mocks.invoke.mockImplementation((name, _args, nextOptions) => {
			if (name !== command) return Promise.resolve(null);
			options = nextOptions;
			return new Promise<void>((resolve, reject) => {
				complete = resolve;
				fail = reject;
			});
		});
		renderWorkspaceList();
		const user = userEvent.setup();
		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.click(screen.getByRole("menuitem", { name: "Delete" }));
		await user.click(screen.getByRole("button", { name: "Delete" }));
		expect(screen.getByText("Deleting...")).toBeInTheDocument();
		expect(options).toBeUndefined();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
		await user.click(screen.getByRole("button", { name: "Deleting..." }));
		expect(
			mocks.invoke.mock.calls.filter(([name]) => name === command),
		).toHaveLength(1);
		await act(async () => {
			if (outcome === "success") complete();
			else fail(new Error("削除が拒否されました"));
		});
		if (outcome === "success") {
			expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
			expect(mocks.refreshWorktrees).toHaveBeenCalled();
		} else {
			expect(screen.getByRole("alert")).toHaveTextContent(
				"削除が拒否されました",
			);
			expect(screen.getByRole("button", { name: "Delete" })).toBeEnabled();
		}
	},
);

it("削除の応答待ちは閉じず完了後に別の対象を開く", async () => {
	const first = makeBranch();
	const next = { ...first, name: "other", worktree_path: "/repo/other" };
	mocks.worktreeBranches = [first, next];
	let options: unknown;
	let complete!: () => void;
	mocks.invoke.mockImplementation((command, _args, nextOptions) => {
		if (command !== "remove_worktree") return Promise.resolve(null);
		options = nextOptions;
		return new Promise<void>((resolve) => {
			complete = resolve;
		});
	});
	renderWorkspaceList();
	const user = userEvent.setup();
	await user.click(
		screen.getByRole("button", { name: "Open menu for feature" }),
	);
	await user.click(screen.getByRole("menuitem", { name: "Delete" }));
	await user.click(screen.getByRole("button", { name: "Delete" }));
	expect(options).toBeUndefined();
	expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
	await user.click(screen.getByRole("button", { name: "Cancel" }));
	expect(screen.getByRole("alertdialog")).toHaveTextContent(
		'Delete workspace for branch "feature"?',
	);
	await act(async () => complete());
	expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
	await user.click(screen.getByRole("button", { name: "Open menu for other" }));
	await user.click(screen.getByRole("menuitem", { name: "Delete" }));
	expect(screen.getByRole("button", { name: "Delete" })).toBeEnabled();
	expect(screen.getByRole("alertdialog")).toHaveTextContent(
		'Delete workspace for branch "other"?',
	);
	expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
