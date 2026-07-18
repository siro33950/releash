import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorktreeBranch } from "@/types/git";
import type {
	WorkspaceSessionHistoryItem,
	WorkspaceTreeItem,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { WorkspaceList } from "./WorkspaceList";

type MockWorkspaceTreeState = {
	nodes: WorkspaceTreeItem[];
	preferredNodeId?: string | null;
	closedSessions?: WorkspaceSessionHistoryItem[];
	workflowHistory?: WorkspaceWorkflowHistoryItem[];
	loading?: boolean;
	error?: string | null;
};

const mocks = vi.hoisted(() => ({
	invoke: vi.fn().mockResolvedValue(null),
	emit: vi.fn().mockResolvedValue(undefined),
	openUrl: vi.fn().mockResolvedValue(undefined),
	archiveSession: vi.fn().mockResolvedValue(undefined),
	restoreSession: vi.fn().mockResolvedValue(undefined),
	getAgentSessionNotice: vi.fn().mockResolvedValue({
		sessionId: "closed-session",
		revision: 1,
		notice: null,
	}),
	refreshTree: vi.fn().mockResolvedValue(undefined),
	refreshWorktrees: vi.fn().mockResolvedValue(undefined),
	treeStateOverrides: new Map<string, MockWorkspaceTreeState>(),
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
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ emit: mocks.emit }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("@/hooks/useSessionStore", () => ({
	archiveSession: mocks.archiveSession,
	getAgentSessionNotice: mocks.getAgentSessionNotice,
	restoreSession: mocks.restoreSession,
}));
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
		return {
			nodes: state.nodes,
			preferredNodeId: state.preferredNodeId ?? null,
			closedSessions: state.closedSessions ?? [],
			workflowHistory: state.workflowHistory ?? [],
			loading: state.loading ?? false,
			error: state.error ?? null,
			refresh: mocks.refreshTree,
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
	id: "4f168b74-f9cf-4d51-9970-81ea281bc983",
	title: "Direct session",
	status: "running",
	contentKind: "session",
	capabilities: { canApprove: false, canClose: true },
	updatedAt: 1,
};

const closedSession: WorkspaceSessionHistoryItem = {
	id: "closed-session",
	worktreePath: "/repo/wt",
	state: "closed",
	createdAt: 1,
	updatedAt: 2,
	firstMessage: "Closed session",
	messageCount: 1,
	permissionMode: "edit",
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
				capabilities: { canApprove: false, canClose: false },
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
						capabilities: { canApprove: false, canClose: false },
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
	};
}

function renderWorkspaceList(
	overrides: Partial<React.ComponentProps<typeof WorkspaceList>> = {},
) {
	const onSelectWorktree = vi.fn();
	const onCreateSession = vi.fn();
	const result = render(
		<WorkspaceList
			repoPaths={["/repo"]}
			selectedRootPath="/repo/wt"
			centerSelection={null}
			onSelectWorktree={onSelectWorktree}
			onCreateSession={onCreateSession}
			onAddRepo={vi.fn()}
			onShowSettings={vi.fn()}
			{...overrides}
		/>,
	);
	return { ...result, onSelectWorktree, onCreateSession };
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) {
		if (typeof mock === "function" && "mockClear" in mock) {
			mock.mockClear();
		}
	}
	mocks.worktreeBranches = [makeBranch()];
	mocks.treeStateOverrides.clear();
	mocks.treeStateOverrides.set("/repo/wt", { nodes: recursiveTree });
	mocks.invoke.mockResolvedValue(null);
});

describe("WorkspaceList", () => {
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
				onCreateSession={vi.fn()}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);
		expect(onSelectWorktree).toHaveBeenCalledTimes(1);
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
				onCreateSession={vi.fn()}
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
		const { onSelectWorktree, onCreateSession, rerender } = renderWorkspaceList(
			{ autoSelectPreferredNode: true },
		);
		await waitFor(() => expect(onSelectWorktree).toHaveBeenCalledTimes(1));

		mocks.worktreeBranches = [{ ...makeBranch(), worktree_path: null }];
		rerender(
			<WorkspaceList
				repoPaths={["/repo"]}
				selectedRootPath={null}
				centerSelection={null}
				autoSelectPreferredNode={true}
				onSelectWorktree={onSelectWorktree}
				onCreateSession={onCreateSession}
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
				onCreateSession={onCreateSession}
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

	it("does not apply a later preferred Node after selection was resolved empty", async () => {
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
				onCreateSession={vi.fn()}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);
		expect(
			screen.getByRole("button", { name: /Architecture review/ }),
		).toHaveAttribute("aria-current", "page");
	});

	it("keeps occurrence order and the selected past occurrence when later executions append", async () => {
		const user = userEvent.setup();
		const occurrenceA1: WorkspaceTreeItem = {
			kind: "node",
			id: "occurrence-a-1",
			title: "A",
			status: "completed",
			contentKind: "session",
			capabilities: { canApprove: false, canClose: false },
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
				onCreateSession={vi.fn()}
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

	it("sends NewSession through the separate creation operation", async () => {
		const user = userEvent.setup();
		const { onCreateSession, onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByRole("button", { name: "Create in feature" }));
		await user.click(screen.getByRole("menuitem", { name: "NewSession" }));

		expect(onCreateSession).toHaveBeenCalledWith("/repo/wt", "feature", "repo");
		expect(onSelectWorktree).not.toHaveBeenCalled();
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

	it("routes SessionHistory restore and archive through stored lifecycle commands", async () => {
		const user = userEvent.setup();
		mocks.treeStateOverrides.set("/repo/wt", {
			nodes: recursiveTree,
			closedSessions: [closedSession],
		});
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.hover(screen.getByRole("menuitem", { name: "SessionHistory" }));
		const restoreItem = await screen.findByRole("menuitem", {
			name: /Closed session/,
		});
		await waitFor(() => expect(mocks.refreshTree).toHaveBeenCalled());
		mocks.refreshTree.mockClear();
		act(() => restoreItem.focus());
		await user.keyboard("{Enter}");

		expect(mocks.restoreSession).toHaveBeenCalledWith("closed-session");
		await waitFor(() => expect(mocks.refreshTree).toHaveBeenCalledOnce());

		await user.click(
			screen.getByRole("button", { name: "Open menu for feature" }),
		);
		await user.hover(screen.getByRole("menuitem", { name: "SessionHistory" }));
		await waitFor(() => expect(mocks.refreshTree).toHaveBeenCalled());
		mocks.refreshTree.mockClear();
		fireEvent.click(
			await screen.findByRole("button", { name: "Archive Closed session" }),
		);

		expect(mocks.archiveSession).toHaveBeenCalledWith("closed-session");
		await waitFor(() => expect(mocks.refreshTree).toHaveBeenCalledOnce());
	});

	it.each([
		["restore", "セッション復元に失敗: unavailable"],
		["archive", "セッションアーカイブに失敗: unavailable"],
	] as const)(
		"shows the originating SessionHistory %s notice without viewable-session registration",
		async (operation, message) => {
			const user = userEvent.setup();
			const otherSession = {
				...closedSession,
				id: "other-session",
				firstMessage: "Other closed session",
			};
			mocks.treeStateOverrides.set("/repo/wt", {
				nodes: recursiveTree,
				closedSessions: [closedSession, otherSession],
			});
			mocks.getAgentSessionNotice.mockResolvedValueOnce({
				sessionId: closedSession.id,
				revision: 1,
				notice: { message },
			});
			if (operation === "restore") {
				mocks.restoreSession.mockRejectedValueOnce(new Error("unavailable"));
			} else {
				mocks.archiveSession.mockRejectedValueOnce(new Error("unavailable"));
			}
			renderWorkspaceList();

			await user.click(
				screen.getByRole("button", { name: "Open menu for feature" }),
			);
			await user.hover(
				screen.getByRole("menuitem", { name: "SessionHistory" }),
			);
			if (operation === "restore") {
				const items = await screen.findAllByRole("menuitem", {
					name: /Closed session/,
				});
				act(() => items[0]?.focus());
				await user.keyboard("{Enter}");
			} else {
				fireEvent.click(
					await screen.findByRole("button", { name: "Archive Closed session" }),
				);
			}

			const alert = await screen.findByRole("alert", { hidden: true });
			expect(screen.getAllByRole("alert", { hidden: true })).toHaveLength(1);
			expect(alert).toHaveAttribute("data-session-id", "closed-session");
			expect(alert).toHaveTextContent(message);
			expect(alert).not.toHaveTextContent("Other closed session");
			expect(mocks.getAgentSessionNotice).toHaveBeenCalledWith(
				"closed-session",
			);
		},
	);

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
