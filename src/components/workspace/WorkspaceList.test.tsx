import { render, screen, waitFor } from "@testing-library/react";
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
	closeSession: vi.fn().mockResolvedValue(undefined),
	restoreSession: vi.fn().mockResolvedValue(undefined),
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
	closeSession: mocks.closeSession,
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
		const { onSelectWorktree, rerender } = renderWorkspaceList();

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
				onSelectWorktree={onSelectWorktree}
				onCreateSession={vi.fn()}
				onAddRepo={vi.fn()}
				onShowSettings={vi.fn()}
			/>,
		);
		expect(onSelectWorktree).toHaveBeenCalledTimes(1);
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
		mocks.invoke.mockResolvedValue({
			id: directNode.id,
			title: directNode.title,
			status: directNode.status,
			capabilities: directNode.capabilities,
			updatedAt: 1,
			content: { kind: "session", sessionId: "session-1" },
		});
		renderWorkspaceList();

		await user.click(
			screen.getByRole("button", { name: "Close Direct session" }),
		);

		expect(mocks.invoke).toHaveBeenCalledWith("get_workspace_node_detail", {
			worktreePath: "/repo/wt",
			nodeId: directNode.id,
		});
		expect(mocks.closeSession).toHaveBeenCalledWith("session-1");
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
