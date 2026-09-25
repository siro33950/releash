import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient as invoke } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { AgentSessionPanel, AgentSessionRoute } from "./AgentSessionPanel";

const states = stateSubscriptions();
vi.mock("@/lib/client", () => ({
	invokeClient: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
	listenClient: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: (props: Record<string, unknown>) => {
		const onTerminalError = props.onTerminalError as
			| ((message: string | null) => void)
			| undefined;
		return (
			<div
				data-testid="provider-terminal"
				data-initialization={String(props.initialization)}
				data-auto-focus={String(props.autoFocus)}
				data-cwd={String(props.cwd)}
				data-owner={JSON.stringify(props.owner)}
			>
				<button
					type="button"
					onClick={() =>
						onTerminalError?.("Terminal resynchronization failed. Try again.")
					}
				>
					Report terminal error
				</button>
				<button type="button" onClick={() => onTerminalError?.(null)}>
					Complete terminal recovery
				</button>
			</div>
		);
	},
}));

const mockInvoke = vi.mocked(invoke);
const resumeAction = (overrides = {}) => ({
	pending: false,
	error: null,
	onResume: vi.fn(),
	...overrides,
});
const session = {
	id: "agent-session-1",
	workspaceIdentity: "/repo",
	providerSessionId: null,
	transcriptRef: null,
	workspaceWorktreePath: "/repo/worktree",
	worktreePath: "/repo-worktrees/.releash-isolated/node-a1",
	provider: "claude" as const,
	treeLocation: {
		treeId: "agent-session-1",
		nodeExecutionId: "agent-session-1",
	},
	lifecycle: "open" as const,
	lastExitAbnormal: false,
	operations: {
		canArchive: true,
		canRestore: false,
		canDelete: false,
	},
};

describe("AgentSessionPanel", () => {
	beforeEach(() => {
		states.clear();
		mockInvoke.mockReset();
	});

	it.each(["attached", "paused"] as const)(
		"%s画面のBackgroundFailuresへAgentSessionのIDを渡す",
		async (outcome) => {
			mockInvoke.mockResolvedValueOnce(outcome);
			states.publish(
				{ kind: "failures", args: [session.id] },
				{
					requiresAttention: true,
					items: [
						{
							operation: "provider_session_title",
							target: session.id,
							classification: "StateRequired",
							message: "session title needs repair",
							count: 1,
							firstObservedMs: 1,
							lastObservedMs: 1,
							requiresAttention: true,
						},
					],
				},
			);
			render(<AgentSessionPanel session={session} />);
			if (outcome === "attached")
				await screen.findByTestId("provider-terminal");
			else await screen.findByText("AgentSession is paused.");
			expect(states.subscribeState).toHaveBeenCalledWith(
				{ kind: "failures", args: [session.id] },
				expect.any(Function),
				expect.any(Function),
			);
			expect(screen.getByText("要対応")).toBeVisible();
			fireEvent.click(screen.getByText("要対応"));
			expect(screen.getByText("session title needs repair")).toBeVisible();
		},
	);

	it("backend open後に既存AgentSession Terminal Surfaceへattachする", async () => {
		mockInvoke.mockResolvedValueOnce("attached");

		render(<AgentSessionPanel session={session} />);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("open_agent_session", {
				agentSessionId: "agent-session-1",
				rows: 24,
				cols: 80,
				callerRequestId: expect.any(String),
			});
		});
		const terminal = await screen.findByTestId("provider-terminal");
		expect(terminal).toHaveAttribute("data-initialization", "attach-existing");
		expect(terminal).toHaveAttribute("data-auto-focus", "true");
		expect(JSON.parse(terminal.getAttribute("data-owner") ?? "{}")).toEqual({
			kind: "session",
			workspacePath: "/repo",
			sessionId: "agent-session-1",
		});
	});

	it("Terminalの失敗文言をalertへ表示し回復成功時に消す", async () => {
		mockInvoke.mockResolvedValueOnce("attached");

		render(<AgentSessionPanel session={session} />);
		fireEvent.click(
			await screen.findByRole("button", { name: "Report terminal error" }),
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Terminal resynchronization failed. Try again.",
		);
		expect(screen.getByTestId("provider-terminal")).toBeVisible();

		fireEvent.click(
			screen.getByRole("button", { name: "Complete terminal recovery" }),
		);

		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});

	it.each([
		["paused", "Provider session is not running."],
		["archived", null],
	] as const)(
		"Terminalの失敗文言を%s画面へ持ち越さない",
		async (lifecycle, expectedAlert) => {
			mockInvoke.mockResolvedValueOnce("attached");
			const { rerender } = render(<AgentSessionPanel session={session} />);
			fireEvent.click(
				await screen.findByRole("button", { name: "Report terminal error" }),
			);
			expect(await screen.findByRole("alert")).toHaveTextContent(
				"Terminal resynchronization failed. Try again.",
			);

			rerender(<AgentSessionPanel session={{ ...session, lifecycle }} />);

			await waitFor(() => {
				expect(
					screen.queryByText("Terminal resynchronization failed. Try again."),
				).not.toBeInTheDocument();
			});
			if (expectedAlert) {
				expect(screen.getByRole("alert")).toHaveTextContent(expectedAlert);
			} else {
				expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			}
		},
	);

	it("PausedではResumeを表示し押すと受け取ったResumeだけを呼ぶ", async () => {
		mockInvoke.mockResolvedValueOnce("paused");
		const action = resumeAction();

		render(
			<AgentSessionPanel
				session={{ ...session, lifecycle: "paused" }}
				resumeAction={action}
			/>,
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Provider session is not running. Resume to retry.",
		);
		fireEvent.click(screen.getByRole("button", { name: "Resume" }));
		expect(action.onResume).toHaveBeenCalledOnce();
		expect(mockInvoke).toHaveBeenCalledTimes(1);
		expect(screen.queryByTestId("provider-terminal")).not.toBeInTheDocument();
	});

	it.each(["open_agent_session", "restore_agent_session"] as const)(
		"%sがGC済みを返しても受け取ったResumeを表示する",
		async (command) => {
			const action = resumeAction();
			if (command === "restore_agent_session") {
				mockInvoke.mockRejectedValueOnce(new Error("archived"));
			}
			mockInvoke.mockResolvedValueOnce("garbage_collected");
			render(
				<AgentSessionPanel
					session={
						command === "restore_agent_session"
							? {
									...session,
									lifecycle: "archived",
									operations: {
										canArchive: false,
										canRestore: true,
										canDelete: true,
									},
								}
							: session
					}
					resumeAction={action}
				/>,
			);
			if (command === "restore_agent_session") {
				await screen.findByRole("alert");
				fireEvent.click(screen.getByRole("button", { name: "Restore" }));
			}
			expect(
				await screen.findByText("AgentSession is no longer available."),
			).toBeVisible();
			expect(mockInvoke).toHaveBeenLastCalledWith(
				command,
				expect.objectContaining({ agentSessionId: session.id }),
			);
			expect(screen.queryByTestId("provider-terminal")).not.toBeInTheDocument();
			fireEvent.click(screen.getByRole("button", { name: "Resume" }));
			expect(action.onResume).toHaveBeenCalledOnce();
			expect(mockInvoke).toHaveBeenCalledTimes(
				command === "open_agent_session" ? 1 : 2,
			);
		},
	);

	it("Resume中は二重に押せず失敗理由を表示する", async () => {
		mockInvoke.mockResolvedValueOnce("paused");
		const { rerender } = render(
			<AgentSessionPanel
				session={{ ...session, lifecycle: "paused" }}
				resumeAction={resumeAction({ pending: true })}
			/>,
		);

		expect(
			await screen.findByRole("button", { name: "Resuming..." }),
		).toBeDisabled();
		rerender(
			<AgentSessionPanel
				session={{ ...session, lifecycle: "paused" }}
				resumeAction={resumeAction({ error: "resume failed" })}
			/>,
		);
		expect(screen.getByText("resume failed")).toBeVisible();
		expect(screen.getByRole("button", { name: "Resume" })).toBeEnabled();
	});

	it("自動resume失敗後はPausedを表示してResumeを待つ", async () => {
		mockInvoke.mockResolvedValueOnce("paused");

		const { rerender } = render(<AgentSessionPanel session={session} />);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Provider session is not running",
		);
		expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
		rerender(
			<AgentSessionPanel
				session={{ ...session, lifecycle: "paused" }}
				resumeAction={resumeAction()}
			/>,
		);
		expect(screen.getByRole("button", { name: "Resume" })).toBeVisible();
		expect(screen.queryByTestId("provider-terminal")).toBeNull();
	});

	it.each([
		[
			{
				code: "AGENT_SESSION_LAUNCH_UNAVAILABLE",
				message: "backend open failed",
			},
			"backend open failed",
		],
		["plain open failed", "plain open failed"],
	])(
		"open失敗からbackendのmessageだけを表示する",
		async (rejection, expected) => {
			mockInvoke.mockRejectedValueOnce(rejection);

			render(<AgentSessionPanel session={session} />);

			expect((await screen.findByRole("alert")).textContent).toBe(expected);
		},
	);

	it("Provider CLI終了でbackendがPausedへ更新した場合もerrorとResumeを表示する", async () => {
		mockInvoke.mockResolvedValueOnce("attached");
		const { rerender } = render(<AgentSessionPanel session={session} />);
		expect(await screen.findByTestId("provider-terminal")).toBeVisible();

		rerender(
			<AgentSessionPanel
				session={{
					...session,
					lifecycle: "paused",
					lastExitAbnormal: true,
				}}
				resumeAction={resumeAction()}
			/>,
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Provider session is not running",
		);
		expect(screen.getByRole("button", { name: "Resume" })).toBeVisible();
	});

	it("Resumeを受け取らないPausedではResumeを表示しない", async () => {
		mockInvoke.mockResolvedValueOnce("paused");

		render(<AgentSessionPanel session={{ ...session, lifecycle: "paused" }} />);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Provider session is not running",
		);
		expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
	});

	it("StandaloneでもTerminal表示の上にArchive操作を置かない", async () => {
		mockInvoke.mockResolvedValueOnce("attached");

		render(<AgentSessionPanel session={session} />);

		expect(await screen.findByTestId("provider-terminal")).toBeVisible();
		expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"archive_agent_session",
			expect.anything(),
		);
	});

	it("Workflow Node所有には個別ArchiveとDeleteを表示しない", async () => {
		mockInvoke.mockResolvedValueOnce("attached");

		render(
			<AgentSessionPanel
				session={{
					...session,
					operations: {
						canArchive: false,
						canRestore: false,
						canDelete: false,
					},
				}}
			/>,
		);

		expect(await screen.findByTestId("provider-terminal")).toBeVisible();
		expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
		expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
	});

	it("Archivedの復帰失敗はArchivedを維持してRestoreとDeleteを表示する", async () => {
		mockInvoke
			.mockRejectedValueOnce(new Error("resume failed"))
			.mockResolvedValueOnce("restored");

		render(
			<AgentSessionPanel
				session={{
					...session,
					lifecycle: "archived",
					operations: {
						canArchive: false,
						canRestore: true,
						canDelete: true,
					},
				}}
			/>,
		);

		expect(await screen.findByRole("alert")).toHaveTextContent("resume failed");
		const restore = screen.getByRole("button", { name: "Restore" });
		expect(screen.getByRole("button", { name: "Delete" })).toBeVisible();
		fireEvent.click(restore);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"restore_agent_session",
				expect.objectContaining({ agentSessionId: "agent-session-1" }),
			);
		});
		expect(await screen.findByText("AgentSession is paused.")).toBeVisible();
		expect(screen.queryByTestId("provider-terminal")).toBeNull();
	});
});

describe("AgentSessionRoute", () => {
	const target = { kind: "agent-session", args: ["agent-session-1"] } as const;
	const publish = (
		value: import("@/generated/client_types").AgentSessionItemDto | null,
	) => states.publish({ ...target, args: [...target.args] }, value);
	beforeEach(() => {
		states.clear();
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValue("attached");
	});
	it("作成済みattachmentは購読の初期値と再Openを待たずTerminalへattachする", () => {
		const consumed = vi.fn();
		render(
			<StrictMode>
				<AgentSessionRoute
					agentSessionId="agent-session-1"
					initialAttachment={{
						agentSessionId: "agent-session-1",
						workspaceIdentity: "/repo",
						worktreePath: "/repo/worktree",
						workspaceWorktreePath: "/repo/worktree",
						provider: "claude",
					}}
					onInitialSessionConsumed={consumed}
				/>
			</StrictMode>,
		);
		expect(screen.getByTestId("provider-terminal")).toBeVisible();
		expect(states.subscribeState).toHaveBeenCalledWith(
			target,
			expect.any(Function),
			expect.any(Function),
		);
		expect(mockInvoke).not.toHaveBeenCalled();
		expect(consumed).toHaveBeenCalledWith("agent-session-1");
		act(() => publish(session));
		expect(mockInvoke).not.toHaveBeenCalled();
	});
	it("購読から届いたsessionをOpenし更新と削除を取り直し無しで表示する", async () => {
		render(
			<AgentSessionRoute
				agentSessionId="agent-session-1"
				resumeAction={resumeAction()}
			/>,
		);
		expect(screen.getByText("Loading AgentSession...")).toBeVisible();
		act(() => publish(session));
		expect(await screen.findByTestId("provider-terminal")).toBeVisible();
		expect(mockInvoke).toHaveBeenCalledExactlyOnceWith(
			"open_agent_session",
			expect.objectContaining({ agentSessionId: "agent-session-1" }),
		);
		act(() => publish({ ...session, lifecycle: "paused" }));
		expect(screen.getByRole("button", { name: "Resume" })).toBeVisible();
		expect(screen.queryByTestId("provider-terminal")).toBeNull();
		act(() => publish(null));
		expect(
			screen.getByText("AgentSession is no longer available."),
		).toBeVisible();
		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});
	it("存在しないsessionでも受け取ったResume操作を提供する", () => {
		publish(null);
		const action = resumeAction();
		render(
			<AgentSessionRoute
				agentSessionId="agent-session-1"
				resumeAction={action}
			/>,
		);
		expect(
			screen.getByText("AgentSession is no longer available."),
		).toBeVisible();
		fireEvent.click(screen.getByRole("button", { name: "Resume" }));
		expect(action.onResume).toHaveBeenCalledOnce();
		expect(mockInvoke).not.toHaveBeenCalled();
	});
	it("Restore操作後の状態は購読から届き再Openしない", async () => {
		publish({
			...session,
			lifecycle: "archived",
			operations: { canArchive: false, canRestore: true, canDelete: true },
		});
		mockInvoke
			.mockRejectedValueOnce(new Error("archived"))
			.mockResolvedValue("restored");
		render(
			<AgentSessionRoute
				agentSessionId="agent-session-1"
				resumeAction={resumeAction()}
			/>,
		);
		fireEvent.click(await screen.findByRole("button", { name: "Restore" }));
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith(
				"restore_agent_session",
				expect.objectContaining({ agentSessionId: "agent-session-1" }),
			),
		);
		act(() => publish({ ...session, lifecycle: "paused" }));
		expect(screen.getByRole("button", { name: "Resume" })).toBeVisible();
		expect(screen.queryByTestId("provider-terminal")).toBeNull();
		expect(
			mockInvoke.mock.calls.filter(([name]) => name === "open_agent_session"),
		).toHaveLength(1);
	});
	it("別sessionの値は表示対象に混ざらない", async () => {
		publish(session);
		const { rerender } = render(
			<AgentSessionRoute agentSessionId="agent-session-1" />,
		);
		await screen.findByTestId("provider-terminal");
		rerender(<AgentSessionRoute agentSessionId="other" />);
		act(() => publish({ ...session, lifecycle: "archived" }));
		expect(screen.getByText("Loading AgentSession...")).toBeVisible();
		act(() => states.publish({ kind: "agent-session", args: ["other"] }, null));
		expect(
			screen.getByText("AgentSession is no longer available."),
		).toBeVisible();
	});
});
