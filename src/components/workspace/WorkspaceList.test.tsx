import { render, screen, waitFor } from "@testing-library/react";
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
					title: "Deploy workflow",
					status: "running",
					updatedAt: 1100,
					children: [
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
					children: [],
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

		expect(screen.getByText("Build step")).toBeInTheDocument();
		await user.click(screen.getByText("Deploy workflow"));

		expect(screen.queryByText("Build step")).not.toBeInTheDocument();
		expect(onSelectWorktree).not.toHaveBeenCalled();
	});

	it("emits a workflowRun selection when a WorkflowSession row is clicked", async () => {
		const user = userEvent.setup();
		const { onSelectWorktree } = renderWorkspaceList();

		await user.click(screen.getByText("Build step"));

		expect(onSelectWorktree).toHaveBeenCalledWith(
			"/repo/wt",
			"feature",
			"repo",
			{
				kind: "workflowRun",
				worktreePath: "/repo/wt",
				runId: "run-1",
				focus: {
					sessionId: "step-build",
					stepName: "build",
					runIndex: 1,
				},
			},
		);
	});

	it("opens the Worktree menu with history, PR link, and delete actions", async () => {
		const user = userEvent.setup();
		renderWorkspaceList();

		await user.click(screen.getByLabelText("Open menu for feature"));

		expect(await screen.findByText("SessionHistory")).toBeInTheDocument();
		expect(screen.getByText("WorkflowHistory")).toBeInTheDocument();
		expect(screen.getByText("PRリンク")).toBeInTheDocument();
		expect(screen.getByText("削除")).toBeInTheDocument();
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
