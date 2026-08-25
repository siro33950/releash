import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
	WorkspaceNodeDetail,
	WorkspaceNodeStatus,
	WorkspaceNodeStatusClassification,
} from "@/types/workspace-tree";
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
vi.mock("@/components/panels/AgentSessionPanel", async () => {
	const { useState } = await import("react");
	return {
		AgentSessionRoute: (props: Record<string, unknown>) => {
			// 実装は initialAttachment を mount 時の state として固定するため、
			// 再マウントされたかどうかを同じ形で観測する。
			const [mountedAttachment] = useState(props.initialAttachment ?? null);
			mocks.agentSessionRoute({ ...props, mountedAttachment });
			return <div data-testid="agent-session-route" />;
		},
	};
});

function sessionDetail(
	id: string,
	sessionId: string | null = `session-${id}`,
): WorkspaceNodeDetail {
	return {
		id,
		title: `Session ${id}`,
		status: "running",
		statusClassification: "active",
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
				kind: "session",
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

	it("created Sessionの既存attachmentを共通Node表示へ引き継ぐ", () => {
		mocks.detailState.detail = sessionDetail(
			"created-session",
			"agent-session-created",
		);
		const onInitialSessionConsumed = vi.fn();
		const initialAttachment = {
			agentSessionId: "agent-session-created",
			workspaceIdentity: "/repo",
			worktreePath: "/repo",
			provider: "codex",
		};

		render(
			<NodeContentView
				worktreePath="/repo"
				nodeId="created-session"
				initialSessionAttachment={initialAttachment}
				onInitialSessionConsumed={onInitialSessionConsumed}
			/>,
		);

		expect(mocks.agentSessionRoute).toHaveBeenCalledWith(
			expect.objectContaining({
				agentSessionId: "agent-session-created",
				initialAttachment,
				onInitialSessionConsumed,
			}),
		);
	});

	it("Session Nodeを切り替えたとき前のNodeのattachmentを持ち越さない", () => {
		const initialAttachment = {
			agentSessionId: "agent-session-a",
			workspaceIdentity: "/repo",
			worktreePath: "/repo",
			provider: "codex",
		};
		mocks.detailState.detail = sessionDetail("node-a", "agent-session-a");

		const { rerender } = render(
			<NodeContentView
				worktreePath="/repo"
				nodeId="node-a"
				initialSessionAttachment={initialAttachment}
			/>,
		);

		expect(mocks.agentSessionRoute).toHaveBeenLastCalledWith(
			expect.objectContaining({
				agentSessionId: "agent-session-a",
				mountedAttachment: initialAttachment,
			}),
		);

		mocks.detailState.detail = sessionDetail("node-b", "agent-session-b");
		rerender(
			<NodeContentView
				worktreePath="/repo"
				nodeId="node-b"
				initialSessionAttachment={initialAttachment}
			/>,
		);

		expect(mocks.agentSessionRoute).toHaveBeenLastCalledWith(
			expect.objectContaining({
				agentSessionId: "agent-session-b",
				initialAttachment: undefined,
				mountedAttachment: null,
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

	it("does not mount a Session surface until a session is attached", () => {
		mocks.detailState.detail = {
			...sessionDetail("pending", null),
		};
		renderView("pending");

		expect(screen.getByText("Session unavailable.")).toBeVisible();
		expect(mocks.agentSessionRoute).not.toHaveBeenCalled();
	});

	it("reports a missing Session as unavailable", () => {
		mocks.detailState.detail = {
			...sessionDetail("missing", null),
			status: "failed",
			statusClassification: "failure",
		};
		renderView("missing");

		expect(screen.getByText("Session unavailable.")).toBeVisible();
		expect(mocks.agentSessionRoute).not.toHaveBeenCalled();
	});

	it("renders masked Command, status, exit code, duration, stdout, and stderr", () => {
		mocks.detailState.detail = {
			id: "command-node",
			title: "Run checks",
			status: "failed",
			statusClassification: "failure",
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

	it("does not expose attempt or internal execution identities", () => {
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
		expect(screen.queryByText(/attempt 3/i)).not.toBeInTheDocument();
		expect(screen.queryByText(/item 2 child 1/i)).not.toBeInTheDocument();
		expect(screen.queryByText("checkpoint-internal")).not.toBeInTheDocument();
	});

	it("uses the backend status as the Node status tooltip", () => {
		mocks.detailState.detail = {
			...sessionDetail("failed-session"),
			status: "failed",
			statusClassification: "failure",
			errorReason: "Agent process exited unexpectedly",
		};
		renderView("failed-session");

		expect(screen.getByTitle("failed")).toBeVisible();
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

	it.each<[WorkspaceNodeStatus, WorkspaceNodeStatusClassification, string]>([
		["running", "active", "lucide-loader-circle"],
		["waiting", "attention", "lucide-clock"],
		["failed", "failure", "lucide-triangle-alert"],
		["paused", "idle", "lucide-circle"],
		["completed", "idle", "lucide-circle-check"],
		["aborted", "idle", "lucide-ban"],
	])(
		"uses the existing %s shape and never pulses in the detail pane",
		(status, statusClassification, expectedShapeClass) => {
			mocks.detailState.detail = {
				...sessionDetail(`${status}-shape`),
				status,
				statusClassification,
			};

			renderView(`${status}-shape`);

			const icon = screen.getByTitle(status).querySelector("svg");
			expect(icon).toHaveClass(expectedShapeClass);
			expect(icon).not.toHaveClass("animate-pulse");
		},
	);

	it.each<
		[
			WorkspaceNodeStatus,
			WorkspaceNodeStatusClassification,
			string,
			string,
			string,
		]
	>([
		[
			"running",
			"active",
			"lucide-loader-circle",
			"text-blue-600",
			"dark:text-blue-300",
		],
		[
			"waiting",
			"attention",
			"lucide-clock",
			"text-yellow-600",
			"dark:text-yellow-300",
		],
		[
			"failed",
			"failure",
			"lucide-triangle-alert",
			"text-red-600",
			"dark:text-red-300",
		],
		[
			"paused",
			"idle",
			"lucide-circle",
			"text-green-600",
			"dark:text-green-300",
		],
	])(
		"colors the %s detail shape from the %s classification without pulsing",
		(status, statusClassification, shapeClass, lightClass, darkClass) => {
			mocks.detailState.detail = {
				...sessionDetail(`${statusClassification}-color`),
				status,
				statusClassification,
			};

			renderView(`${statusClassification}-color`);

			const icon = screen.getByTitle(status).querySelector("svg");
			expect(icon).toHaveClass(shapeClass, lightClass, darkClass);
			expect(icon).not.toHaveClass("animate-pulse");
		},
	);

	it("shows backend-owned error and recovery reasons without deriving them in the TUI", () => {
		mocks.detailState.detail = {
			...sessionDetail("paused-session"),
			status: "paused",
			statusClassification: "failure",
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
		expect(screen.queryByText("Attempt 2")).not.toBeInTheDocument();
		expect(screen.getByText("Artifact submitted")).toBeVisible();
		await user.click(screen.getByRole("button", { name: "Retry" }));
		expect(mocks.retryWorkspaceNode).toHaveBeenCalledWith({
			worktreePath: "/repo",
			nodeId: "waiting-stop",
		});
	});
});
