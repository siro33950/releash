import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceNodeDetail } from "@/types/workspace-tree";
import { NodeContentView } from "./NodeContentView";

const mocks = vi.hoisted(() => ({
	detailState: {
		detail: null as WorkspaceNodeDetail | null,
		loading: false,
		error: null as string | null,
		missingNodeId: null as string | null,
	},
	agentSessionRoute: vi.fn(),
	approveWorkspaceNode: vi.fn().mockResolvedValue(null),
	retryWorkspaceNode: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/hooks/useWorkspaceNodeDetail", () => ({
	useWorkspaceNodeDetail: () => mocks.detailState,
	approveWorkspaceNode: (...args: unknown[]) =>
		mocks.approveWorkspaceNode(...args),
	retryWorkspaceNode: (...args: unknown[]) => mocks.retryWorkspaceNode(...args),
}));
vi.mock("@/components/panels/AgentSessionPanel", () => ({
	AgentSessionRoute: (props: Record<string, unknown>) => {
		mocks.agentSessionRoute(props);
		return <div data-testid="agent-session-route" />;
	},
}));

function sessionDetail(
	id: string,
	sessionId: string | null = `session-${id}`,
): WorkspaceNodeDetail {
	return {
		id,
		title: `Session ${id}`,
		status: "running",
		submitReceived: false,
		stopReceived: false,
		hasArtifact: false,
		capabilities: { canApprove: false, canRetry: false, canClose: false },
		updatedAt: 1,
		content: { kind: "agentSession", sessionId },
	};
}

function renderView(nodeId = "node") {
	return render(
		<NodeContentView worktreePath="/repo" nodeId={nodeId} theme="light" />,
	);
}

beforeEach(() => {
	mocks.detailState.detail = null;
	mocks.detailState.loading = false;
	mocks.detailState.error = null;
	mocks.detailState.missingNodeId = null;
	mocks.agentSessionRoute.mockClear();
	mocks.approveWorkspaceNode.mockClear();
	mocks.retryWorkspaceNode.mockClear();
});

describe("NodeContentView", () => {
	it("reports only an authoritative missing result for the selected Node", () => {
		const onNodeMissing = vi.fn();
		mocks.detailState.missingNodeId = "old-node";
		const { rerender } = render(
			<NodeContentView
				worktreePath="/repo"
				nodeId="new-node"
				onNodeMissing={onNodeMissing}
			/>,
		);
		expect(onNodeMissing).not.toHaveBeenCalled();

		mocks.detailState.missingNodeId = "new-node";
		rerender(
			<NodeContentView
				worktreePath="/repo"
				nodeId="new-node"
				onNodeMissing={onNodeMissing}
			/>,
		);
		expect(onNodeMissing).toHaveBeenCalledOnce();
		expect(onNodeMissing).toHaveBeenCalledWith("/repo", "new-node");
	});

	it("Workflow Session NodeはAgentSession Terminalを表示する", () => {
		mocks.detailState.detail = {
			...sessionDetail("workflow-session", "agent-session-1"),
			content: {
				kind: "agentSession",
				sessionId: "agent-session-1",
			},
		} as unknown as WorkspaceNodeDetail;

		renderView("workflow-session");

		expect(screen.getByTestId("agent-session-route")).toBeVisible();
		expect(mocks.agentSessionRoute).toHaveBeenCalledWith(
			expect.objectContaining({
				agentSessionId: "agent-session-1",
				theme: "light",
			}),
		);
	});

	it("provides a bounded flex column so the AgentSession terminal can scroll", () => {
		mocks.detailState.detail = sessionDetail("standalone");
		renderView("standalone");

		const contentBoundary = screen.getByTestId(
			"agent-session-route",
		).parentElement;
		expect(contentBoundary).toHaveClass(
			"flex",
			"min-h-0",
			"flex-1",
			"flex-col",
			"overflow-hidden",
		);
	});

	it("shows a queued Session without mounting chat until a session is attached", () => {
		mocks.detailState.detail = {
			...sessionDetail("queued", null),
			status: "queued",
		};
		renderView("queued");

		expect(screen.getByText("This session has not started yet.")).toBeVisible();
		expect(mocks.agentSessionRoute).not.toHaveBeenCalled();
	});

	it("reports a missing non-queued Session as unavailable", () => {
		mocks.detailState.detail = {
			...sessionDetail("missing", null),
			status: "failed",
		};
		renderView("missing");

		expect(screen.getByText("Session unavailable.")).toBeVisible();
		expect(
			screen.queryByText("This session has not started yet."),
		).not.toBeInTheDocument();
		expect(mocks.agentSessionRoute).not.toHaveBeenCalled();
	});

	it("renders masked Command, status, exit code, duration, stdout, and stderr", () => {
		mocks.detailState.detail = {
			id: "command-node",
			title: "Run checks",
			status: "failed",
			submitReceived: false,
			stopReceived: false,
			hasArtifact: false,
			capabilities: { canApprove: false, canRetry: false, canClose: false },
			updatedAt: 2,
			content: {
				kind: "command",
				displayCommand: "curl -H 'token: ********' https://example.test",
				result: {
					exitCode: 7,
					duration: 145,
					stdout: "masked stdout",
					stderr: "masked stderr",
				},
			},
		};
		renderView("command-node");

		expect(screen.getByTestId("workspace-command")).toHaveTextContent(
			"token: ********",
		);
		expect(screen.getByText("failed")).toBeVisible();
		expect(screen.getByText("7")).toBeVisible();
		expect(screen.getByText("145 ms")).toBeVisible();
		expect(screen.getByTestId("workspace-command-stdout")).toHaveTextContent(
			"masked stdout",
		);
		expect(screen.getByTestId("workspace-command-stderr")).toHaveTextContent(
			"masked stderr",
		);
	});

	it("shows the public attempt without exposing internal execution identities", () => {
		mocks.detailState.detail = {
			...sessionDetail("public-title"),
			title: "Public title",
			executionId: "execution-internal-uuid",
			nodeExecutionId: "node-execution-internal-uuid",
			attempt: 3,
			fanoutParent: "item 2 child 1",
			resumeFromNode: "checkpoint-internal",
		} as WorkspaceNodeDetail;
		renderView("public-title");

		expect(screen.getByText("Public title")).toBeVisible();
		expect(
			screen.queryByText("execution-internal-uuid"),
		).not.toBeInTheDocument();
		expect(
			screen.queryByText("node-execution-internal-uuid"),
		).not.toBeInTheDocument();
		expect(screen.getByText(/attempt 3/i)).toBeVisible();
		expect(screen.queryByText(/item 2 child 1/i)).not.toBeInTheDocument();
		expect(screen.queryByText("checkpoint-internal")).not.toBeInTheDocument();
	});

	it("uses the Error reason as the Node status tooltip", () => {
		mocks.detailState.detail = {
			...sessionDetail("failed-session"),
			status: "error",
			errorReason: "Agent process exited unexpectedly",
		};
		renderView("failed-session");

		expect(
			screen.getByTitle("Agent process exited unexpectedly"),
		).toBeVisible();
	});

	it("falls back to the status as the Node status tooltip", () => {
		mocks.detailState.detail = {
			...sessionDetail("running-session"),
			status: "running",
			errorReason: "stale reason",
		};
		renderView("running-session");

		expect(screen.getByTitle("running")).toBeVisible();
	});

	it("shows backend-owned error and recovery reasons without deriving them in the TUI", () => {
		mocks.detailState.detail = {
			...sessionDetail("paused-session"),
			status: "paused",
			errorReason: "Node activation failed",
			recoveryReason: "Provider session must be recovered",
		};
		renderView("paused-session");

		expect(screen.getByText("Node activation failed")).toBeVisible();
		expect(
			screen.getByText("Provider session must be recovered"),
		).toBeVisible();
	});

	it("shows and executes Approve only from backend capability", async () => {
		const user = userEvent.setup();
		mocks.detailState.detail = {
			...sessionDetail("approval"),
			capabilities: { canApprove: true, canRetry: false, canClose: false },
		};
		renderView("approval");

		await user.click(screen.getByRole("button", { name: "Approve" }));
		expect(mocks.approveWorkspaceNode).toHaveBeenCalledWith({
			worktreePath: "/repo",
			nodeId: "approval",
		});
	});

	it("shows the backend-owned signal wait and executes Retry only from backend capability", async () => {
		const user = userEvent.setup();
		mocks.detailState.detail = {
			...sessionDetail("waiting-stop"),
			attempt: 2,
			submitReceived: true,
			stopReceived: false,
			waitingFor: "stop",
			hasArtifact: true,
			capabilities: {
				canApprove: false,
				canClose: false,
				canRetry: true,
			},
		};
		renderView("waiting-stop");

		expect(
			screen.getByText("Submit received · waiting for Stop"),
		).toBeVisible();
		expect(screen.getByText("Attempt 2")).toBeVisible();
		expect(screen.getByText("Artifact submitted")).toBeVisible();
		await user.click(screen.getByRole("button", { name: "Retry" }));
		expect(mocks.retryWorkspaceNode).toHaveBeenCalledWith({
			worktreePath: "/repo",
			nodeId: "waiting-stop",
		});
	});
});
