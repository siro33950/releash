import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
	WorkspaceTreeSelectionSnapshot,
	WorkspaceTreeSnapshot,
} from "@/types/workspace-tree";

const SELECTED_NODE_ID = "selected-workflow-node";
const FALLBACK_NODE_ID = "fallback-session-node";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn().mockResolvedValue(vi.fn()),
	emit: vi.fn().mockResolvedValue(undefined),
	openWorktreeTab: vi.fn(),
	initFromCwd: vi.fn(),
	addRepo: vi.fn(),
	removeRepo: vi.fn(),
	updateSettings: vi.fn(),
	updateTheme: vi.fn(),
	refreshWorktrees: vi.fn().mockResolvedValue(undefined),
	archiveCommitted: false,
	postArchiveSnapshot: null as unknown,
	reconciliationFailuresRemaining: 0,
	workspaceSelectionInvalidated: null as
		| ((worktreePath: string, nodeId: string) => void)
		| null,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
	emit: mocks.emit,
	listen: mocks.listen,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/hooks/useSettings", () => ({
	useSettings: () => ({
		settings: { autoUpdate: false, theme: "dark" },
		updateSettings: mocks.updateSettings,
		updateTheme: mocks.updateTheme,
	}),
}));
vi.mock("@/hooks/useUpdateChecker", () => ({
	useUpdateChecker: () => null,
}));
vi.mock("@/hooks/useWorkspaceNavigation", () => ({
	useWorkspaceNavigation: () => ({
		worktrees: [{ id: "wt", rootPath: "/repo/wt" }],
		selectedWorktreeId: "wt",
		openWorktreeTab: mocks.openWorktreeTab,
	}),
}));
vi.mock("@/hooks/useRepoList", () => ({
	useRepoList: () => ({
		repoPaths: ["/repo"],
		addRepo: mocks.addRepo,
		removeRepo: mocks.removeRepo,
		initFromCwd: mocks.initFromCwd,
	}),
}));
vi.mock("@/hooks/useMenuEvents", () => ({ useMenuEvents: vi.fn() }));
vi.mock("@/hooks/useSessionStore", () => ({
	archiveSession: vi.fn().mockResolvedValue(undefined),
	getAgentSessionNotice: vi.fn().mockResolvedValue({
		sessionId: "closed-session",
		revision: 1,
		notice: null,
	}),
	listClosedSessions: vi.fn().mockResolvedValue([]),
	restoreSession: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/hooks/useWorkflowConfig", () => ({
	useWorkflowConfig: () => ({ workflows: [], loading: false, error: null }),
}));
vi.mock("@/hooks/useWorktreeList", () => ({
	useWorktreeList: () => ({
		branches: [
			{
				name: "feature",
				is_main_worktree: false,
				worktree_path: "/repo/wt",
				dirty_count: 0,
				is_merged: false,
				ahead: 0,
				behind: 0,
				has_upstream: false,
				base_ahead: 0,
			},
		],
		loading: false,
		refresh: mocks.refreshWorktrees,
	}),
}));
vi.mock("@/components/UpdateDialog", () => ({ UpdateDialog: () => null }));
vi.mock("@/components/panels/SettingsModal", () => ({
	SettingsModal: () => null,
}));
vi.mock("@/screens/MainLayout", () => ({
	MainLayout: ({
		leftNav,
		selectedRootPath,
		centerSelectionByWorktree,
	}: {
		leftNav: React.ReactNode;
		selectedRootPath: string | null;
		centerSelectionByWorktree?: Record<string, { nodeId: string } | null>;
	}) => {
		const centerSelection = selectedRootPath
			? (centerSelectionByWorktree?.[selectedRootPath] ?? null)
			: null;
		mocks.workspaceSelectionInvalidated = (
			leftNav as React.ReactElement<{
				onWorkspaceSelectionInvalidated: (
					worktreePath: string,
					nodeId: string,
				) => void;
			}>
		).props.onWorkspaceSelectionInvalidated;
		return (
			<div>
				{leftNav}
				<div data-testid="center-node">{centerSelection?.nodeId ?? "none"}</div>
			</div>
		);
	},
}));

const initialSnapshot: WorkspaceTreeSnapshot = {
	nodes: [
		{
			kind: "node",
			id: FALLBACK_NODE_ID,
			title: "Fallback session",
			status: "active",
			contentKind: "session",
			capabilities: { canApprove: false, canRetry: false, canClose: true },
			pastAttempts: [],
			pastAttemptsCollapsed: false,
			updatedAt: 1,
		},
		{
			kind: "sequence",
			id: "archivable-workflow",
			title: "Archivable workflow",
			status: "idle",
			workflowCapabilities: {
				canStop: false,
				canResume: false,
				canAbort: false,
				canArchive: true,
			},
			children: [
				{
					kind: "node",
					id: SELECTED_NODE_ID,
					title: "Selected workflow Node",
					status: "idle",
					contentKind: "session",
					capabilities: { canApprove: false, canRetry: false, canClose: false },
					pastAttempts: [],
					pastAttemptsCollapsed: false,
					updatedAt: 2,
				},
			],
			updatedAt: 2,
		},
	],
	archivedSessions: [],
	preferredNodeId: SELECTED_NODE_ID,
};

const fallbackSnapshot: WorkspaceTreeSnapshot = {
	nodes: [
		{
			kind: "node",
			id: FALLBACK_NODE_ID,
			title: "Fallback session",
			status: "active",
			contentKind: "session",
			capabilities: { canApprove: false, canRetry: false, canClose: true },
			pastAttempts: [],
			pastAttemptsCollapsed: false,
			updatedAt: 3,
		},
	],
	archivedSessions: [],
	preferredNodeId: FALLBACK_NODE_ID,
};

function reconciliation(
	snapshot: WorkspaceTreeSnapshot,
	selectionInSnapshot: boolean,
): WorkspaceTreeSelectionSnapshot {
	return {
		snapshot,
		reconciliation: { selectionInSnapshot },
	};
}

function snapshotContainsNode(
	nodes: WorkspaceTreeSnapshot["nodes"],
	selectedNodeId: string,
): boolean {
	return nodes.some((item) => {
		if (item.kind === "node") return item.id === selectedNodeId;
		return snapshotContainsNode(item.children, selectedNodeId);
	});
}

const { default: App } = await import("./App");

beforeEach(() => {
	vi.clearAllMocks();
	mocks.archiveCommitted = false;
	mocks.postArchiveSnapshot = fallbackSnapshot;
	mocks.reconciliationFailuresRemaining = 0;
	mocks.workspaceSelectionInvalidated = null;
	mocks.invoke.mockImplementation((command: string, args?: unknown) => {
		if (command === "get_application_startup_outcome") {
			return Promise.resolve({ type: "ready" });
		}
		if (command === "get_cwd" || command === "get_main_repo_path") {
			return Promise.resolve("/repo");
		}
		if (command === "list_worktrees") return Promise.resolve([]);
		if (command === "list_workspace_worktree_nodes") {
			return Promise.resolve(
				mocks.archiveCommitted
					? (mocks.postArchiveSnapshot as WorkspaceTreeSnapshot)
					: initialSnapshot,
			);
		}
		if (command === "get_workspace_tree_selection_reconciliation") {
			if (mocks.reconciliationFailuresRemaining > 0) {
				mocks.reconciliationFailuresRemaining -= 1;
				return Promise.reject(new Error("temporary reconciliation failure"));
			}
			const snapshot = mocks.archiveCommitted
				? (mocks.postArchiveSnapshot as WorkspaceTreeSnapshot)
				: initialSnapshot;
			const selectedNodeId = (args as { selectedNodeId: string })
				.selectedNodeId;
			return Promise.resolve(
				reconciliation(
					snapshot,
					snapshotContainsNode(snapshot.nodes, selectedNodeId),
				),
			);
		}
		if (command === "list_workspace_workflow_history") {
			return Promise.resolve([]);
		}
		if (command === "archive_workspace_workflow_execution") {
			mocks.archiveCommitted = true;
			return Promise.resolve(null);
		}
		return Promise.resolve(null);
	});
});

describe("App Workspace Archive selection reconciliation", () => {
	it("retries a failed Archive read on the next refresh and falls back to the snapshot preferred Node", async () => {
		const user = userEvent.setup();
		mocks.reconciliationFailuresRemaining = 1;
		render(<App />);
		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				SELECTED_NODE_ID,
			),
		);
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) =>
					command === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(0);

		await user.click(
			screen.getByRole("button", { name: "Archive Archivable workflow" }),
		);
		await waitFor(() =>
			expect(
				mocks.invoke.mock.calls.filter(
					([command]) =>
						command === "get_workspace_tree_selection_reconciliation",
				),
			).toHaveLength(1),
		);
		expect(screen.getByTestId("center-node")).toHaveTextContent(
			SELECTED_NODE_ID,
		);

		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo/wt" },
				}),
			);
		});

		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				FALLBACK_NODE_ID,
			),
		);
		expect(mocks.invoke).toHaveBeenCalledWith(
			"archive_workspace_workflow_execution",
			{ worktreePath: "/repo/wt", executionId: "archivable-workflow" },
		);
		const archiveCallIndex = mocks.invoke.mock.calls.findIndex(
			([command]) => command === "archive_workspace_workflow_execution",
		);
		expect(
			mocks.invoke.mock.calls
				.slice(archiveCallIndex + 1)
				.some(
					([command, args]) =>
						command === "get_workspace_tree_selection_reconciliation" &&
						(args as { selectedNodeId?: string }).selectedNodeId ===
							SELECTED_NODE_ID,
				),
		).toBe(true);
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) =>
					command === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(2);

		const listCallsAfterSuccess = mocks.invoke.mock.calls.filter(
			([command]) => command === "list_workspace_worktree_nodes",
		).length;
		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo/wt" },
				}),
			);
		});
		await waitFor(() =>
			expect(
				mocks.invoke.mock.calls.filter(
					([command]) => command === "list_workspace_worktree_nodes",
				).length,
			).toBeGreaterThan(listCallsAfterSuccess),
		);
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) =>
					command === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(2);
		expect(mocks.archiveCommitted).toBe(true);
	});

	it("retries a failed Archive read and becomes unselected when the snapshot has no preferred Node", async () => {
		const user = userEvent.setup();
		mocks.postArchiveSnapshot = {
			nodes: [],
			archivedSessions: [],
			preferredNodeId: null,
		};
		mocks.reconciliationFailuresRemaining = 1;
		render(<App />);
		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				SELECTED_NODE_ID,
			),
		);
		await user.click(
			screen.getByRole("button", { name: "Archive Archivable workflow" }),
		);
		await waitFor(() =>
			expect(
				mocks.invoke.mock.calls.filter(
					([command]) =>
						command === "get_workspace_tree_selection_reconciliation",
				),
			).toHaveLength(1),
		);
		expect(screen.getByTestId("center-node")).toHaveTextContent(
			SELECTED_NODE_ID,
		);
		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo/wt" },
				}),
			);
		});

		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent("none"),
		);
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) =>
					command === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(2);
		expect(mocks.archiveCommitted).toBe(true);
	});

	it("keeps the selection when it remains in the accepted Archive snapshot", async () => {
		const user = userEvent.setup();
		mocks.postArchiveSnapshot = initialSnapshot;
		render(<App />);
		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				SELECTED_NODE_ID,
			),
		);

		await user.click(
			screen.getByRole("button", { name: "Archive Archivable workflow" }),
		);
		await waitFor(() =>
			expect(
				mocks.invoke.mock.calls.filter(
					([command]) =>
						command === "get_workspace_tree_selection_reconciliation",
				),
			).toHaveLength(1),
		);
		expect(screen.getByTestId("center-node")).toHaveTextContent(
			SELECTED_NODE_ID,
		);
	});

	it("ignores a delayed invalidation callback for a Node that is no longer selected", async () => {
		const user = userEvent.setup();
		render(<App />);
		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				SELECTED_NODE_ID,
			),
		);

		await user.click(
			screen.getByRole("button", { name: "Fallback session, active" }),
		);
		await waitFor(() =>
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				FALLBACK_NODE_ID,
			),
		);
		act(() => {
			mocks.workspaceSelectionInvalidated?.("/repo/wt", SELECTED_NODE_ID);
		});

		expect(screen.getByTestId("center-node")).toHaveTextContent(
			FALLBACK_NODE_ID,
		);
	});
});
