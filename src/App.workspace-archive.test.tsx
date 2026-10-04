import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
	WorkspaceTreeSelectionSnapshot,
	WorkspaceTreeSnapshot,
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
	states.publish("startup-outcome", { type: "ready" });
	states.publish(
		"workspaces",
		workspaceListSnapshot(initialSnapshot, "/repo/wt"),
	);
	states.publish("startup-repository", {
		path: "/repo",
		branch: "main",
		repositoryName: "repo",
	});
	states.publish({ kind: "worktrees", args: ["/repo"] }, []);
	mocks.archiveCommitted = false;
	mocks.postArchiveSnapshot = fallbackSnapshot;
	mocks.reconciliationFailuresRemaining = 0;
	mocks.workspaceSelectionInvalidated = null;
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "subscribe_daemon_status") {
			(
				args as { channel: { onmessage?: (value: unknown) => void } }
			).channel.onmessage?.({ phase: "ready" });
			return;
		}
		return { type: "ready" };
	});
	mocks.invoke.mockImplementation((command: string) => {
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

	it("archive後の選択の購読失敗をWorkspacesの行に表示する", async () => {
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
				expect.any(Function),
			),
		);
		act(() =>
			states.fail(
				{ kind: "selection", args: ["/repo/wt", SELECTED_NODE_ID] },
				new Error("selection unavailable"),
			),
		);
		expect(
			await screen.findByText(/selection unavailable/),
		).toBeInTheDocument();
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

it("daemonが選んだ起動worktreeをそのまま一度だけ表示する", async () => {
	states.publish("startup-repository", {
		path: "/repo/only",
		branch: "feature",
		repositoryName: "repo",
	});
	states.publish({ kind: "worktrees", args: ["/repo"] }, [
		{
			name: "only",
			path: "/repo/only",
			branch: "feature",
			is_main: true,
			is_locked: false,
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
	expect(states.subscribeState).toHaveBeenCalledWith(
		"startup-repository",
		expect.any(Function),
		expect.any(Function),
	);
	expect(states.firstState).not.toHaveBeenCalled();
});

it("起動repositoryの読取失敗を画面に表示する", async () => {
	states.publish("workspaces", {
		status: { loaded: true, error: null, state: "ready" },
		repositories: [],
	});
	states.publish("startup-repository", null);
	states.fail("startup-repository", new Error("startup repository unreadable"));
	render(<App />);
	await screen.findByText("startup repository unreadable");
	expect(mocks.openWorktreeTab).not.toHaveBeenCalled();
});

it("repositoryの外の起動は自動表示も失敗表示も行わない", async () => {
	states.publish("workspaces", {
		status: { loaded: true, error: null, state: "ready" },
		repositories: [],
	});
	states.publish("startup-repository", null);
	render(<App />);
	await waitFor(() =>
		expect(states.subscribeState).toHaveBeenCalledWith(
			"startup-repository",
			expect.any(Function),
			expect.any(Function),
		),
	);
	expect(mocks.openWorktreeTab).not.toHaveBeenCalled();
	expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
