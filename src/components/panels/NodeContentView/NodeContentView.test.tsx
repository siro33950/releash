import { act, render, screen } from "@testing-library/react";
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
	resumeWorkspaceSessionNode: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/hooks/useWorkspaceNodeDetail", () => ({
	useWorkspaceNodeDetail: () => mocks.detailState,
	approveWorkspaceNode: (...args: unknown[]) =>
		mocks.approveWorkspaceNode(...args),
	retryWorkspaceNode: (...args: unknown[]) => mocks.retryWorkspaceNode(...args),
	resumeWorkspaceSessionNode: (...args: unknown[]) =>
		mocks.resumeWorkspaceSessionNode(...args),
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
		processPresence: "unknown",
		submitReceived: false,
		stopReceived: false,
		hasArtifact: false,
		capabilities: {
			canRename: false,
			canApprove: false,
			canRetry: false,
			canResumeSession: false,
		},
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
	mocks.resumeWorkspaceSessionNode.mockClear();
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
				showResumeAction: false,
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
			workspaceWorktreePath: "/repo",
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
			workspaceWorktreePath: "/repo",
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
			status: "running",
			statusClassification: "attention",
		};
		renderView("missing");

		expect(screen.getByText("Session unavailable.")).toBeVisible();
		expect(mocks.agentSessionRoute).not.toHaveBeenCalled();
	});

	it("renders masked Command, status, exit code, duration, stdout, and stderr", () => {
		mocks.detailState.detail = {
			id: "command-node",
			title: "Run checks",
			status: "completed",
			statusClassification: "idle",
			processPresence: "unknown",
			submitReceived: false,
			stopReceived: false,
			hasArtifact: false,
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: false,
				canResumeSession: false,
			},
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
		expect(screen.getByText("completed")).toBeVisible();
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
	});

	it("uses the backend status as the Node status tooltip", () => {
		mocks.detailState.detail = {
			...sessionDetail("absent-session"),
			status: "running",
			statusClassification: "attention",
			errorReason: "Agent process exited unexpectedly",
		};
		renderView("absent-session");

		expect(screen.getByTitle("running")).toBeVisible();
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
			"completed",
			"idle",
			"lucide-circle-check",
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

	it("shows and executes Approve only from backend capability", async () => {
		const user = userEvent.setup();
		mocks.detailState.detail = {
			...sessionDetail("approval"),
			capabilities: {
				canRename: false,
				canApprove: true,
				canRetry: false,
				canResumeSession: false,
			},
		};
		renderView("approval");

		await user.click(screen.getByRole("button", { name: "Approve" }));
		expect(mocks.approveWorkspaceNode).toHaveBeenCalledWith({
			worktreePath: "/repo",
			nodeId: "approval",
		});
	});

	it("shows the backend-owned signal wait without making Session Retry available", async () => {
		mocks.detailState.detail = {
			...sessionDetail("waiting-stop"),
			submitReceived: true,
			stopReceived: false,
			waitingFor: "stop",
			hasArtifact: true,
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: false,
				canResumeSession: false,
			},
		};
		renderView("waiting-stop");

		expect(
			screen.getByText("Submit received · waiting for Stop"),
		).toBeVisible();
		expect(screen.queryByText("Attempt 2")).not.toBeInTheDocument();
		expect(screen.getByText("Artifact submitted")).toBeVisible();
		expect(
			screen.queryByRole("button", { name: "Retry" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Resume" }),
		).not.toBeInTheDocument();
	});
});

it.each(["running", "completed", "aborted"] as const)(
	"%sのNodeでArtifactなしでも隔離branchとpathを表示する",
	(status) => {
		mocks.detailState.detail = {
			...sessionDetail("isolated"),
			status,
			worktree: {
				branch: "releash/isolated/node-a1",
				path: "/repo-worktrees/.releash-isolated/node-a1",
			},
		};
		renderView("isolated");
		expect(screen.getByText("releash/isolated/node-a1")).toBeVisible();
		expect(
			screen.getByText("/repo-worktrees/.releash-isolated/node-a1"),
		).toBeVisible();
	},
);

it.each(["success", "failure"])(
	"承認は通信状態を表示せず応答の%sを反映する",
	async (outcome) => {
		const { act } = await import("@testing-library/react");
		let complete!: () => void;
		let fail!: (error: Error) => void;
		mocks.approveWorkspaceNode.mockImplementationOnce(() => {
			return new Promise<void>((resolve, reject) => {
				complete = resolve;
				fail = reject;
			});
		});
		mocks.detailState.detail = {
			...sessionDetail("approval"),
			capabilities: {
				canRename: false,
				canApprove: true,
				canRetry: false,
				canResumeSession: false,
			},
		};
		renderView("approval");
		await userEvent.click(screen.getByRole("button", { name: "Approve" }));
		expect(screen.getByText("Approving...")).toBeInTheDocument();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Approving..." })).toBeDisabled();
		await userEvent.click(screen.getByRole("button", { name: "Approving..." }));
		expect(mocks.approveWorkspaceNode).toHaveBeenCalledTimes(1);
		await act(async () => {
			if (outcome === "success") complete();
			else fail(new Error("承認が拒否されました"));
		});
		expect(
			screen.queryByText(/操作結果を確認できません/),
		).not.toBeInTheDocument();
		if (outcome === "failure")
			expect(screen.getByText("承認が拒否されました")).toBeInTheDocument();
	},
);

it.each(["success", "failure"])(
	"Retryは通信状態を表示せず応答の%sを反映する",
	async (outcome) => {
		const { act } = await import("@testing-library/react");
		let complete!: () => void;
		let fail!: (error: Error) => void;
		mocks.retryWorkspaceNode.mockImplementationOnce(() => {
			return new Promise<void>((resolve, reject) => {
				complete = resolve;
				fail = reject;
			});
		});
		mocks.detailState.detail = {
			...sessionDetail("retry"),
			processPresence: "confirmed_absent",
			content: { kind: "command", displayCommand: "echo ok", result: null },
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: true,
				canResumeSession: false,
			},
		};
		renderView("retry");
		await userEvent.click(screen.getByRole("button", { name: "Retry" }));
		expect(screen.getByText("Retrying...")).toBeInTheDocument();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "元の操作の結果を確認" }),
		).not.toBeInTheDocument();
		const button = screen.getByRole("button", { name: "Retrying..." });
		expect(button).toBeDisabled();
		await userEvent.click(button);
		expect(mocks.retryWorkspaceNode).toHaveBeenCalledTimes(1);
		await act(async () => {
			if (outcome === "success") complete();
			else fail(new Error("再試行が拒否されました"));
		});
		expect(
			screen.queryByText(/操作結果を確認できません/),
		).not.toBeInTheDocument();
		if (outcome === "failure")
			expect(screen.getByRole("alert")).toHaveTextContent(
				"再試行が拒否されました",
			);
		else expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
	},
);

it.each(["live", "confirmed_absent", "unknown"] as const)(
	"プロセス在否 %s をNode状態と別に表示する",
	(presence) => {
		mocks.detailState.detail = {
			...sessionDetail("presence"),
			processPresence: presence,
		};
		renderView("presence");
		expect(screen.getByTitle("running")).toBeVisible();
		expect(
			screen.getByText(
				presence === "live"
					? "Process running"
					: presence === "confirmed_absent"
						? "No process"
						: "Process unknown",
			),
		).toBeVisible();
	},
);

it("Session Resume をbackend capabilityから表示し新しい入口へ送る", async () => {
	const user = userEvent.setup();
	const detail = sessionDetail("resume", null);
	mocks.detailState.detail = {
		...detail,
		processPresence: "confirmed_absent",
		capabilities: { ...detail.capabilities, canResumeSession: true },
	};
	renderView("resume");
	expect(
		screen.queryByRole("button", { name: "Retry" }),
	).not.toBeInTheDocument();
	await user.click(screen.getByRole("button", { name: "Resume" }));
	expect(mocks.resumeWorkspaceSessionNode).toHaveBeenCalledWith({
		worktreePath: "/repo",
		nodeId: "resume",
	});
});

it("Session Resume 中は二重送信を防ぎ失敗理由を表示する", async () => {
	const user = userEvent.setup();
	const detail = sessionDetail("resume-error", null);
	mocks.detailState.detail = {
		...detail,
		capabilities: { ...detail.capabilities, canResumeSession: true },
	};
	let reject!: (error: Error) => void;
	mocks.resumeWorkspaceSessionNode.mockReturnValueOnce(
		new Promise((_, fail) => {
			reject = fail;
		}),
	);
	renderView("resume-error");
	await user.click(screen.getByRole("button", { name: "Resume" }));
	expect(screen.getByRole("button", { name: "Resuming..." })).toBeDisabled();
	expect(mocks.resumeWorkspaceSessionNode).toHaveBeenCalledTimes(1);
	await act(async () => {
		reject(new Error("provider recovery failed"));
	});
	expect(screen.getByText("provider recovery failed")).toBeVisible();
	expect(screen.getByRole("button", { name: "Resume" })).toBeEnabled();
});
