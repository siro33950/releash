import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorktreeBranch } from "@/types/git";
import type {
	WorkspaceSessionHistoryItem,
	WorkspaceTreeNode,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { WorkspaceList } from "./WorkspaceList";

type MockWorkspaceTreeState = {
	nodes: WorkspaceTreeNode[];
	closedSessions?: WorkspaceSessionHistoryItem[];
	workflowHistory?: WorkspaceWorkflowHistoryItem[];
	loading?: boolean;
	error?: string | null;
};

const mocks = vi.hoisted(() => ({
	invoke: vi.fn().mockResolvedValue([]),
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

vi.mock("react-resizable-panels", () => {
	const Panel = ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	);
	const Group = ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	);
	const Separator = () => <div />;
	return { Panel, Group, Separator };
});
vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({
	emit: mocks.emit,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: mocks.openUrl,
}));
vi.mock("@/hooks/useSessionStore", () => ({
	archiveSession: mocks.archiveSession,
	closeSession: mocks.closeSession,
	restoreSession: mocks.restoreSession,
}));
vi.mock("@/hooks/useWorkflowConfig", () => ({
	useWorkflowConfig: () => ({
		workflows: [
			{
				name: "release",
				description: "Release workflow",
				builtin: false,
				is_running: false,
			},
		],
		loading: false,
		error: null,
	}),
}));
vi.mock("@/hooks/useWorkspaceTreeNodes", () => ({
	useWorkspaceTreeNodes: (worktreePath: string) => {
		const override = mocks.treeStateOverrides.get(worktreePath);
		if (override) {
			return {
				nodes: override.nodes,
				closedSessions: override.closedSessions ?? [],
				workflowHistory: override.workflowHistory ?? [],
				loading: override.loading ?? false,
				error: override.error ?? null,
				refresh: mocks.refreshTree,
			};
		}
		if (worktreePath !== "/repo/wt") {
			return {
				nodes: [],
				closedSessions: [],
				workflowHistory: [],
				loading: false,
				error: null,
				refresh: mocks.refreshTree,
			};
		}
		return {
			nodes: [
				{
					kind: "session",
					id: "new-s",
					worktreePath: "/repo/wt",
					title: "NewSession",
					state: "idle",
					updatedAt: 1000,
					workflowStepSession: false,
					agentState: null,
				},
				{
					kind: "session",
					id: "direct-1",
					worktreePath: "/repo/wt",
					title: "Direct session",
					state: "active",
					updatedAt: 1000,
					workflowStepSession: false,
					agentState: "done",
				},
				{
					kind: "workflow",
					runId: "run-1",
					worktreePath: "/repo/wt",
					workflowName: "release",
					title: "Deploy workflow",
					status: "running",
					canStop: true,
					updatedAt: 1100,
					steps: [
						{
							kind: "step",
							id: "run-1:step-build",
							runId: "run-1",
							worktreePath: "/repo/wt",
							title: "Build step",
							status: "running",
							stepType: "agent",
							updatedAt: 1100,
							sessions: [
								{
									kind: "session",
									id: "step-build",
									worktreePath: "/repo/wt",
									title: "Build step",
									state: "active",
									updatedAt: 1100,
									workflowStepSession: true,
									stepName: "build",
									runIndex: 1,
									agentState: "running",
								},
							],
						},
					],
				},
				{
					kind: "workflow",
					runId: "run-waiting",
					worktreePath: "/repo/wt",
					workflowName: "waiting-flow",
					title: "Waiting workflow",
					status: "waiting",
					canStop: true,
					updatedAt: 1101,
					steps: [],
				},
				{
					kind: "workflow",
					runId: "run-completed",
					worktreePath: "/repo/wt",
					workflowName: "completed-flow",
					title: "Completed workflow",
					status: "completed",
					canStop: false,
					updatedAt: 1102,
					steps: [],
				},
				{
					kind: "workflow",
					runId: "run-failed",
					worktreePath: "/repo/wt",
					workflowName: "failed-flow",
					title: "Failed workflow",
					status: "failed",
					canStop: false,
					updatedAt: 1103,
					steps: [],
				},
				{
					kind: "workflow",
					runId: "run-aborted",
					worktreePath: "/repo/wt",
					workflowName: "aborted-flow",
					title: "Aborted workflow",
					status: "aborted",
					canStop: false,
					updatedAt: 1104,
					steps: [],
				},
			],
			closedSessions: [
				{
					id: "closed-1",
					worktreePath: "/repo/wt",
					state: "closed",
					createdAt: 1,
					updatedAt: 2,
					firstMessage: "Closed history session",
					messageCount: 1,
					agentSessionId: null,
					contextCarry: null,
					permissionMode: "edit",
					planMode: false,
					permissionProfileId: null,
					backendId: null,
					workflowStepSession: false,
				},
			],
			workflowHistory: [
				{
					runId: "archived-run",
					worktreePath: "/repo/wt",
					title: "Archived workflow",
					status: "completed",
					updatedAt: 3,
					archivedAt: 4,
					archiveReason: "manual",
				},
			],
			loading: false,
			error: null,
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
vi.mock("@/hooks/useWorktreeSessionStatuses", () => ({
	useWorktreeSessionStatuses: () => new Map(),
}));
vi.mock("@/hooks/useWorktreeStepStatuses", () => ({
	workflowStepStatusKey: (
		executionId: string,
		stepName: string,
		runIndex?: number | null,
	) => `${executionId}:${stepName}:${runIndex ?? 1}`,
	useWorktreeStepStatuses: () => ({
		steps: new Map([["run-1:Build step:1", "waiting"]]),
		workflows: new Map([["run-1", "failed"]]),
	}),
}));

function makeBranch(overrides: Partial<WorktreeBranch>): WorktreeBranch {
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
		...overrides,
	};
}

const defaultWorktreeBranches = [
	makeBranch({
		name: "main",
		is_main_worktree: true,
		worktree_path: "/repo/main",
	}),
	makeBranch({
		name: "feature",
		is_main_worktree: false,
		worktree_path: "/repo/wt",
		has_pr: true,
		pr_number: 12,
		pr_url: "https://example.test/pr/12",
	}),
];

function setSingleWorktree(
	worktreePath: string,
	state: MockWorkspaceTreeState,
	name = "empty",
) {
	mocks.worktreeBranches = [
		makeBranch({
			name,
			worktree_path: worktreePath,
		}),
	];
	mocks.treeStateOverrides.set(worktreePath, state);
}

function renderWorkspaceList(
	props: Partial<React.ComponentProps<typeof WorkspaceList>> = {},
) {
	const onSelectWorktree = vi.fn();
	const renderResult = render(
		<WorkspaceList
			repoPaths={["/repo"]}
			selectedRootPath="/repo/wt"
			centerSelection={null}
			onSelectWorktree={onSelectWorktree}
			onAddRepo={vi.fn()}
			onShowSettings={vi.fn()}
			{...props}
		/>,
	);
	return { onSelectWorktree, ...renderResult };
}

beforeEach(() => {
	for (const mock of [
		mocks.invoke,
		mocks.emit,
		mocks.openUrl,
		mocks.archiveSession,
		mocks.closeSession,
		mocks.restoreSession,
		mocks.refreshTree,
		mocks.refreshWorktrees,
	]) {
		mock.mockClear();
	}
	mocks.treeStateOverrides.clear();
	mocks.worktreeBranches = defaultWorktreeBranches.map((branch) => ({
		...branch,
	}));
	mocks.invoke.mockResolvedValue([]);
});

describe("WorkspaceList", () => {
	it("highlights the created NewSession row when selected", () => {
		renderWorkspaceList({
			centerSelection: {
				kind: "agentSession",
				worktreePath: "/repo/wt",
				sessionId: "new-s",
			},
		});

		expect(screen.getByText("NewSession").closest("button")).toHaveAttribute(
			"aria-current",
			"page",
		);
	});

	it("renders Home and worktree icon variants", () => {
		renderWorkspaceList();

		expect(screen.getByLabelText("Main repository")).toBeInTheDocument();
		expect(screen.getByLabelText("Worktree")).toBeInTheDocument();
	});

	it("toggles a Worktree row without changing CenterSelection", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		expect(screen.getByText("Direct session")).toBeInTheDocument();
		await user.click(screen.getByTestId("worktree-item-feature"));

		expect(screen.queryByText("Direct session")).not.toBeInTheDocument();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("shows an empty placeholder for an expanded Worktree with no sessions or workflows", () => {
		setSingleWorktree("/repo/empty", {
			nodes: [],
			loading: false,
			error: null,
		});
		renderWorkspaceList({ selectedRootPath: "/repo/empty" });

		const placeholder = screen.getByText("No sessions or workflows");
		expect(placeholder).toBeInTheDocument();
		expect(placeholder).toHaveClass("text-muted-foreground");
		expect(placeholder).toHaveStyle({ paddingLeft: "26px" });
		expect(screen.queryByText("Direct session")).not.toBeInTheDocument();
		expect(screen.queryByText("release")).not.toBeInTheDocument();
	});

	it("does not show the empty placeholder when the Worktree has nodes", () => {
		mocks.worktreeBranches = [
			makeBranch({
				name: "feature",
				worktree_path: "/repo/wt",
				has_pr: true,
				pr_number: 12,
				pr_url: "https://example.test/pr/12",
			}),
		];
		renderWorkspaceList();

		expect(screen.getByText("Direct session")).toBeInTheDocument();
		expect(screen.getByText("release")).toBeInTheDocument();
		expect(
			screen.queryByText("No sessions or workflows"),
		).not.toBeInTheDocument();
	});

	it("keeps the empty placeholder hidden while Worktree nodes are loading", () => {
		setSingleWorktree(
			"/repo/loading",
			{
				nodes: [],
				loading: true,
				error: null,
			},
			"loading",
		);
		const { container } = renderWorkspaceList({
			selectedRootPath: "/repo/loading",
		});

		expect(container.querySelector(".animate-spin")).toBeInTheDocument();
		expect(
			screen.queryByText("No sessions or workflows"),
		).not.toBeInTheDocument();
	});

	it("keeps the empty placeholder hidden when Worktree node loading fails with no nodes", () => {
		setSingleWorktree(
			"/repo/error",
			{
				nodes: [],
				loading: false,
				error: "Failed to load workspace tree",
			},
			"error",
		);
		renderWorkspaceList({ selectedRootPath: "/repo/error" });

		expect(
			screen.getByText("Failed to load workspace tree"),
		).toBeInTheDocument();
		expect(
			screen.queryByText("No sessions or workflows"),
		).not.toBeInTheDocument();
	});

	it("does not render the empty placeholder for a collapsed Worktree", async () => {
		const user = userEvent.setup();
		setSingleWorktree("/repo/empty", {
			nodes: [],
			loading: false,
			error: null,
		});
		renderWorkspaceList({ selectedRootPath: "/repo/empty" });

		expect(screen.getByText("No sessions or workflows")).toBeInTheDocument();
		await user.click(screen.getByTestId("worktree-item-empty"));

		expect(
			screen.queryByText("No sessions or workflows"),
		).not.toBeInTheDocument();
	});

	it("emits an agentSession selection when a Session row is clicked", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByText("Direct session"));

		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "agentSession",
				worktreePath: "/repo/wt",
				sessionId: "direct-1",
			},
		);
	});

	it("toggles a Workflow parent without changing CenterSelection", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		expect(screen.getByText("release")).toBeInTheDocument();
		expect(screen.queryByText("Deploy workflow")).not.toBeInTheDocument();
		expect(screen.getByText("Build step")).toBeInTheDocument();
		await user.click(screen.getByText("release"));

		expect(screen.queryByText("Build step")).not.toBeInTheDocument();
		expect(screen.queryByText("Running")).not.toBeInTheDocument();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("emits a workflowStep selection when a Workflow Step row is clicked", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByText("Build step"));

		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "workflowStep",
				worktreePath: "/repo/wt",
				runId: "run-1",
				stepId: "run-1:step-build",
				stepName: "Build step",
			},
		);
	});

	it("opens the Workflow row menu and stops a stoppable Workflow", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByLabelText("Open menu for release"));
		const stop = await screen.findByText("Stop");
		await user.click(stop);

		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith("abort_workflow", {
				runId: "run-1",
			});
		});
		expect(mocks.refreshTree).toHaveBeenCalled();
		expect(screen.queryByRole("alert")).toBeNull();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("shows an error when stopping a Workflow fails", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();
		mocks.invoke.mockRejectedValueOnce("abort denied");

		await user.click(screen.getByLabelText("Open menu for release"));
		const stop = await screen.findByText("Stop");
		await user.click(stop);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Stop workflow failed: abort denied",
		);
		expect(mocks.refreshTree).not.toHaveBeenCalled();
	});

	it("keeps Stop enabled for a waiting approval Workflow", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(screen.getByLabelText("Open menu for waiting-flow"));
		const stop = await screen.findByRole("menuitem", { name: /Stop/ });
		expect(stop).not.toHaveAttribute("aria-disabled", "true");
		await user.click(stop);

		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith("abort_workflow", {
				runId: "run-waiting",
			});
		});
	});

	it("uses live representative statuses for Step and Workflow rows", () => {
		renderWorkspaceList();

		expect(screen.getAllByTitle("waiting").length).toBeGreaterThanOrEqual(2);
		expect(screen.getAllByTitle("failed").length).toBeGreaterThanOrEqual(2);
		expect(screen.getAllByTitle("completed").length).toBeGreaterThanOrEqual(1);
	});

	it("uses a fixed Workflow icon for Workflow rows and keeps Step row status icons", () => {
		renderWorkspaceList();

		const workflowButton = screen.getByText("release").closest("button");
		const stepButton = screen.getByText("Build step").closest("button");

		expect(
			workflowButton?.querySelector('[title="failed"] svg.lucide-workflow'),
		).toBeInTheDocument();
		expect(
			stepButton?.querySelector('[title="waiting"] svg.lucide-clock'),
		).toBeInTheDocument();
		expect(stepButton?.querySelector("svg.lucide-workflow")).toBeNull();
	});

	it("does not enable Stop from a live representative status on a terminal Workflow", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(screen.getByLabelText("Open menu for completed-flow"));
		const stop = await screen.findByRole("menuitem", { name: /Stop/ });
		expect(stop).toHaveAttribute("aria-disabled", "true");
		fireEvent.click(stop);

		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"abort_workflow",
			expect.anything(),
		);
	});

	it.each([
		["completed", "completed-flow"],
		["failed", "failed-flow"],
		["aborted", "aborted-flow"],
	])("disables Stop for a %s Workflow and ignores clicks", async (_status, workflowLabel) => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(screen.getByLabelText(`Open menu for ${workflowLabel}`));
		const stop = await screen.findByRole("menuitem", { name: /Stop/ });
		expect(stop).toHaveAttribute("aria-disabled", "true");
		fireEvent.click(stop);

		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"abort_workflow",
			expect.anything(),
		);
	});

	it("archives a Workflow row without changing CenterSelection", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByLabelText("Archive release"));

		await waitFor(() => {
			expect(mocks.invoke).toHaveBeenCalledWith(
				"archive_workspace_workflow_run",
				{
					worktreePath: "/repo/wt",
					runId: "run-1",
				},
			);
		});
		expect(mocks.refreshTree).toHaveBeenCalled();
		expect(screen.queryByRole("alert")).toBeNull();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("shows an error when archiving a Workflow fails", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();
		mocks.invoke.mockRejectedValueOnce("archive denied");

		await user.click(screen.getByLabelText("Archive release"));

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Archive workflow failed: archive denied",
		);
		expect(mocks.refreshTree).not.toHaveBeenCalled();
	});

	it("opens the Worktree menu with history, PR link, and delete actions", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(screen.getByLabelText("Open menu for feature"));

		expect(await screen.findByText("SessionHistory")).toBeInTheDocument();
		expect(screen.getByText("WorkflowHistory")).toBeInTheDocument();
		expect(screen.getByText("PR Link")).toBeInTheDocument();
		expect(screen.getByText("Delete")).toBeInTheDocument();
	});

	it("closes a Session from the hover action without selecting the row", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByLabelText("Close Direct session"));

		await waitFor(() => {
			expect(mocks.closeSession).toHaveBeenCalledWith("direct-1");
		});
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("does not render status text, relative time, or more controls", () => {
		renderWorkspaceList();

		expect(screen.queryByText("Open")).not.toBeInTheDocument();
		expect(screen.queryByText("Running")).not.toBeInTheDocument();
		expect(screen.queryByText("Closed")).not.toBeInTheDocument();
		expect(screen.queryByText("1日")).not.toBeInTheDocument();
		expect(screen.queryByText("もっと表示する")).not.toBeInTheDocument();
	});

	it("renders the header Add Worktree action", () => {
		renderWorkspaceList();

		expect(
			screen.getByRole("button", { name: "Add Worktree" }),
		).toBeInTheDocument();
	});

	it("shows the NewWorkflow submenu from the Worktree create menu", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(screen.getByLabelText("Create in feature"));

		await user.hover(await screen.findByText("NewWorkflow"));

		expect(await screen.findByText("release")).toBeInTheDocument();
	});
});
