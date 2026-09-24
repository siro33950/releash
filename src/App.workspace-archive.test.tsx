import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
	WorkspaceTreeSelectionSnapshotDto as WorkspaceTreeSelectionSnapshot,
	WorkspaceTreeSnapshotDto as WorkspaceTreeSnapshot,
} from "@/generated/client_types";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { workspaceListSnapshot } from "@/test/workspaceList";

const states = stateSubscriptions();

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

vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: mocks.invoke,
	listenClient: mocks.listen,
	completeClientRestoration: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
	firstState: (...args: Parameters<typeof states.firstState>) =>
		states.firstState(...args),
}));

import { invoke } from "@tauri-apps/api/core";

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
			processPresence: "unknown",
			id: FALLBACK_NODE_ID,
			title: "Fallback session",
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
		{
			kind: "sequence",
			id: "archivable-workflow",
			title: "Archivable workflow",
			status: "idle",
			workflowCapabilities: {
				canAbort: false,
				canArchive: true,
			},
			children: [
				{
					kind: "node",
					processPresence: "unknown",
					id: SELECTED_NODE_ID,
					title: "Selected workflow Node",
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
			processPresence: "unknown",
			id: FALLBACK_NODE_ID,
			title: "Fallback session",
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
	states.clear();
	states.publish(
		"workspaces",
		workspaceListSnapshot(initialSnapshot, "/repo/wt"),
	);
	states.publish("startup-repository", "/repo");
	states.publish({ kind: "worktrees", args: ["/repo"] }, []);
	mocks.archiveCommitted = false;
	mocks.postArchiveSnapshot = fallbackSnapshot;
	mocks.reconciliationFailuresRemaining = 0;
	mocks.workspaceSelectionInvalidated = null;
	vi.mocked(invoke).mockImplementation(async (command) =>
		command === "get_daemon_status" ? { phase: "ready" } : { type: "ready" },
	);
	mocks.invoke.mockImplementation((command: string) => {
		if (command === "get_application_startup_outcome") {
			return Promise.resolve({ type: "ready" });
		}
		if (command === "archive_workspace_workflow_execution") {
			mocks.archiveCommitted = true;
			return Promise.resolve(null);
		}
		return Promise.resolve(null);
	});
});

describe("App Workspace Archive selection reconciliation", () => {
	it.each([
		["preferred Node", fallbackSnapshot, FALLBACK_NODE_ID],
		["no preferred Node", { nodes: [], archivedSessions: [] }, "none"],
		["selection remains", initialSnapshot, SELECTED_NODE_ID],
	] satisfies [string, WorkspaceTreeSnapshot, string][])(
		"購読の照合結果で選択を更新する: %s",
		async (_label, snapshot, expected) => {
			const user = userEvent.setup();
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
				expect(states.subscribeState).toHaveBeenCalledWith(
					{ kind: "selection", args: ["/repo/wt", SELECTED_NODE_ID] },
					expect.any(Function),
				),
			);
			expect(screen.getByTestId("center-node")).toHaveTextContent(
				SELECTED_NODE_ID,
			);
			act(() => {
				states.publish(
					"workspaces",
					workspaceListSnapshot(snapshot, "/repo/wt"),
				);
				states.publish(
					{ kind: "selection", args: ["/repo/wt", SELECTED_NODE_ID] },
					reconciliation(
						snapshot,
						snapshotContainsNode(snapshot.nodes, SELECTED_NODE_ID),
					),
				);
			});
			await waitFor(() =>
				expect(screen.getByTestId("center-node")).toHaveTextContent(expected),
			);
			expect(mocks.invoke).toHaveBeenCalledWith(
				"archive_workspace_workflow_execution",
				{ worktreePath: "/repo/wt", executionId: "archivable-workflow" },
			);
			expect(mocks.invoke).not.toHaveBeenCalledWith(
				"refresh_workspaces",
				expect.anything(),
			);
		},
	);

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

it("起動repositoryのworktreeが1件ならそのタブを自動表示する", async () => {
	states.publish({ kind: "worktrees", args: ["/repo"] }, [
		{
			name: "only",
			path: "/repo/only",
			branch: "feature",
			is_main: true,
			is_locked: false,
			dirty_count: 0,
			base_branch: null,
		},
	]);
	render(<App />);
	await waitFor(() =>
		expect(mocks.openWorktreeTab).toHaveBeenCalledWith(
			"/repo/only",
			"feature",
			"repo",
		),
	);
	expect(mocks.initFromCwd).toHaveBeenCalledWith("/repo");
	expect(states.firstState).toHaveBeenCalledWith("startup-repository");
	expect(states.firstState).toHaveBeenCalledWith({
		kind: "worktrees",
		args: ["/repo"],
	});
});
