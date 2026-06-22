import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspaceList } from "./WorkspaceList";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn().mockResolvedValue([]),
	emit: vi.fn().mockResolvedValue(undefined),
	openUrl: vi.fn().mockResolvedValue(undefined),
	archiveSession: vi.fn().mockResolvedValue(undefined),
	closeSession: vi.fn().mockResolvedValue(undefined),
	restoreSession: vi.fn().mockResolvedValue(undefined),
	refreshTree: vi.fn().mockResolvedValue(undefined),
	refreshWorktrees: vi.fn().mockResolvedValue(undefined),
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
					status: "waiting_approval",
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
		branches: [
			{
				name: "main",
				is_main_worktree: true,
				worktree_path: "/repo/main",
				dirty_count: 0,
				is_merged: false,
				ahead: 0,
				behind: 0,
				has_upstream: false,
				base_ahead: 0,
			},
			{
				name: "feature",
				is_main_worktree: false,
				worktree_path: "/repo/wt",
				dirty_count: 0,
				is_merged: false,
				has_pr: true,
				pr_number: 12,
				pr_url: "https://example.test/pr/12",
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
vi.mock("@/hooks/useWorktreeSessionStatuses", () => ({
	useWorktreeSessionStatuses: () => new Map(),
}));

function renderWorkspaceList(
	props: Partial<React.ComponentProps<typeof WorkspaceList>> = {},
) {
	const onSelectWorktree = vi.fn();
	render(
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
	return { onSelectWorktree };
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) {
		mock.mockClear();
	}
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
