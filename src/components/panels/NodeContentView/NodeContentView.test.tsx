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
	boundSessionChat: vi.fn(),
	providerAgentSessionRoute: vi.fn(),
	approveWorkspaceNode: vi.fn().mockResolvedValue(null),
	retryWorkspaceNode: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/hooks/useWorkspaceNodeDetail", () => ({
	useWorkspaceNodeDetail: () => mocks.detailState,
	approveWorkspaceNode: (...args: unknown[]) =>
		mocks.approveWorkspaceNode(...args),
	retryWorkspaceNode: (...args: unknown[]) => mocks.retryWorkspaceNode(...args),
}));
vi.mock("@/components/panels/AgentChatPanel", () => ({
	BoundSessionChat: (props: Record<string, unknown>) => {
		mocks.boundSessionChat(props);
		return (
			<div data-testid="bound-session-chat">
				<div>Conversation transcript</div>
				<textarea aria-label="Message input" />
				<button type="button">Respond permission</button>
			</div>
		);
	},
}));
vi.mock("@/components/panels/ProviderAgentSessionPanel", () => ({
	ProviderAgentSessionRoute: (props: Record<string, unknown>) => {
		mocks.providerAgentSessionRoute(props);
		return <div data-testid="provider-agent-session-route" />;
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
		content: { kind: "session", sessionId },
	};
}

function renderView(nodeId = "node") {
	return render(
		<NodeContentView
			worktreePath="/repo"
			nodeId={nodeId}
			theme="light"
			activeEditorPath="/repo/src/main.ts"
			openEditorPaths={["/repo/src/main.ts"]}
			activeEditorSelection={{
				filePath: "/repo/src/main.ts",
				startLine: 2,
				endLine: 4,
			}}
			registerDropZone={vi.fn()}
			sendMessageRef={{ current: null }}
			onOpenDiffFile={vi.fn()}
		/>,
	);
}

beforeEach(() => {
	mocks.detailState.detail = null;
	mocks.detailState.loading = false;
	mocks.detailState.error = null;
	mocks.detailState.missingNodeId = null;
	mocks.boundSessionChat.mockClear();
	mocks.providerAgentSessionRoute.mockClear();
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
				registerDropZone={vi.fn()}
				onNodeMissing={onNodeMissing}
			/>,
		);
		expect(onNodeMissing).not.toHaveBeenCalled();

		mocks.detailState.missingNodeId = "new-node";
		rerender(
			<NodeContentView
				worktreePath="/repo"
				nodeId="new-node"
				registerDropZone={vi.fn()}
				onNodeMissing={onNodeMissing}
			/>,
		);
		expect(onNodeMissing).toHaveBeenCalledOnce();
		expect(onNodeMissing).toHaveBeenCalledWith("/repo", "new-node");
	});

	it("uses the same complete BoundSessionChat for standalone and Workflow Session Nodes", () => {
		mocks.detailState.detail = sessionDetail(
			"standalone",
			"session-standalone",
		);
		const { rerender } = renderView("standalone");

		expect(screen.getByText("Conversation transcript")).toBeInTheDocument();
		expect(
			screen.getByRole("textbox", { name: "Message input" }),
		).toBeVisible();
		expect(
			screen.getByRole("button", { name: "Respond permission" }),
		).toBeVisible();
		expect(mocks.boundSessionChat).toHaveBeenLastCalledWith(
			expect.objectContaining({
				sessionId: "session-standalone",
				worktreePath: "/repo",
				activeEditorPath: "/repo/src/main.ts",
				openEditorPaths: ["/repo/src/main.ts"],
				activeEditorSelection: {
					filePath: "/repo/src/main.ts",
					startLine: 2,
					endLine: 4,
				},
				dropZoneName: "agent",
			}),
		);

		mocks.detailState.detail = sessionDetail(
			"workflow-session",
			"session-workflow",
		);
		rerender(
			<NodeContentView
				worktreePath="/repo"
				nodeId="workflow-session"
				registerDropZone={vi.fn()}
			/>,
		);
		expect(mocks.boundSessionChat).toHaveBeenLastCalledWith(
			expect.objectContaining({
				sessionId: "session-workflow",
				worktreePath: "/repo",
			}),
		);
	});

	it("Workflow Session NodeはProvider AgentSession Terminalを表示する", () => {
		mocks.detailState.detail = {
			...sessionDetail("workflow-session", "provider-agent-session-1"),
			content: {
				kind: "providerAgentSession",
				sessionId: "provider-agent-session-1",
			},
		} as unknown as WorkspaceNodeDetail;

		renderView("workflow-session");

		expect(screen.getByTestId("provider-agent-session-route")).toBeVisible();
		expect(mocks.providerAgentSessionRoute).toHaveBeenCalledWith(
			expect.objectContaining({
				agentSessionId: "provider-agent-session-1",
				theme: "light",
			}),
		);
		expect(mocks.boundSessionChat).not.toHaveBeenCalled();
	});

	it("provides a bounded flex column so the Session transcript can scroll", () => {
		mocks.detailState.detail = sessionDetail("standalone");
		renderView("standalone");

		const contentBoundary =
			screen.getByTestId("bound-session-chat").parentElement;
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
		expect(mocks.boundSessionChat).not.toHaveBeenCalled();
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
		expect(mocks.boundSessionChat).not.toHaveBeenCalled();
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
