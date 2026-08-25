import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentSessionPanel, AgentSessionRoute } from "./AgentSessionPanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: (props: Record<string, unknown>) => (
		<div
			data-testid="provider-terminal"
			data-initialization={String(props.initialization)}
			data-auto-focus={String(props.autoFocus)}
			data-owner={JSON.stringify(props.owner)}
		/>
	),
}));

const mockInvoke = vi.mocked(invoke);
const session = {
	id: "agent-session-1",
	workspaceIdentity: "/repo",
	worktreePath: "/repo/worktree",
	provider: "claude" as const,
	lifecycle: "open" as const,
	activity: "idle" as const,
	lastExitAbnormal: false,
	operations: {
		canArchive: true,
		canRestore: false,
		canDelete: false,
		canResume: false,
	},
};

describe("AgentSessionPanel", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

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

	it("Pausedは明示Resumeが成功するまでTerminalをattachしない", async () => {
		mockInvoke.mockResolvedValueOnce("paused").mockResolvedValueOnce("resumed");

		render(
			<AgentSessionPanel
				session={{
					...session,
					lifecycle: "paused",
					operations: { ...session.operations, canResume: true },
				}}
			/>,
		);

		const resume = await screen.findByRole("button", { name: "Resume" });
		expect(screen.queryByTestId("provider-terminal")).not.toBeInTheDocument();
		fireEvent.click(resume);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("resume_agent_session", {
				agentSessionId: "agent-session-1",
				rows: 24,
				cols: 80,
				callerRequestId: expect.any(String),
			});
		});
		expect(await screen.findByTestId("provider-terminal")).toBeInTheDocument();
	});

	it("自動resume失敗後はPausedとerrorを表示して明示Resumeを待つ", async () => {
		const onRefresh = vi.fn();
		mockInvoke
			.mockResolvedValueOnce("paused")
			.mockRejectedValueOnce(new Error("resume failed"));

		const { rerender } = render(
			<AgentSessionPanel session={session} onRefresh={onRefresh} />,
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Provider session is not running",
		);
		expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
		expect(onRefresh).toHaveBeenCalledOnce();
		rerender(
			<AgentSessionPanel
				session={{
					...session,
					lifecycle: "paused",
					operations: { ...session.operations, canResume: true },
				}}
				onRefresh={onRefresh}
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: "Resume" }));
		expect(await screen.findByRole("alert")).toHaveTextContent("resume failed");
		expect(screen.getByRole("button", { name: "Resume" })).toBeVisible();
		expect(screen.queryByTestId("provider-terminal")).toBeNull();
	});

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
					operations: { ...session.operations, canResume: true },
				}}
			/>,
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Provider session is not running",
		);
		expect(screen.getByRole("button", { name: "Resume" })).toBeVisible();
	});

	it("provider session identity未確定のPausedではResumeを表示しない", async () => {
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
						canResume: false,
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
						canResume: false,
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
		expect(await screen.findByTestId("provider-terminal")).toBeVisible();
	});
});

describe("AgentSessionRoute", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

	it("作成応答のAgentSessionは再取得と再Openを待たずTerminalへattachする", async () => {
		const onInitialSessionConsumed = vi.fn();
		mockInvoke.mockImplementation((command) => {
			if (command === "get_agent_session") {
				return new Promise(() => {});
			}
			return Promise.reject(new Error(`unexpected command: ${command}`));
		});

		render(
			<StrictMode>
				<AgentSessionRoute
					agentSessionId="agent-session-1"
					initialAttachment={{
						agentSessionId: "agent-session-1",
						workspaceIdentity: "/repo",
						worktreePath: "/repo/worktree",
						provider: "claude",
					}}
					onInitialSessionConsumed={onInitialSessionConsumed}
				/>
			</StrictMode>,
		);

		expect(screen.getByTestId("provider-terminal")).toBeVisible();
		expect(mockInvoke).toHaveBeenCalledWith("get_agent_session", {
			agentSessionId: "agent-session-1",
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"open_agent_session",
			expect.anything(),
		);
		expect(onInitialSessionConsumed).toHaveBeenCalledWith("agent-session-1");
	});

	it("idからbackend read modelを取得してAgentSessionを開く", async () => {
		mockInvoke.mockImplementation((command) => {
			if (command === "get_agent_session") {
				return Promise.resolve(session);
			}
			if (command === "open_agent_session") {
				return Promise.resolve("attached");
			}
			return Promise.reject(new Error(`unexpected command: ${command}`));
		});

		render(<AgentSessionRoute agentSessionId="agent-session-1" />);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_agent_session", {
				agentSessionId: "agent-session-1",
			});
		});
		expect(await screen.findByTestId("provider-terminal")).toBeVisible();
	});

	it("backendにAgentSessionが存在しなければLoadingを終了して不在を表示する", async () => {
		mockInvoke.mockResolvedValueOnce(null);

		render(<AgentSessionRoute agentSessionId="missing-session" />);

		expect(
			await screen.findByText("AgentSession is no longer available."),
		).toBeVisible();
		expect(screen.queryByText("Loading AgentSession...")).toBeNull();
	});

	it("Restore後にbackend read modelを再取得してOpen操作を表示する", async () => {
		const archived = {
			...session,
			lifecycle: "archived" as const,
			operations: {
				canArchive: false,
				canRestore: true,
				canDelete: true,
				canResume: false,
			},
		};
		let getReads = 0;
		let openCalls = 0;
		mockInvoke.mockImplementation((command) => {
			if (command === "get_agent_session") {
				getReads += 1;
				return Promise.resolve(getReads === 1 ? archived : session);
			}
			if (command === "open_agent_session") {
				openCalls += 1;
				return openCalls === 1
					? Promise.reject(new Error("resume failed"))
					: Promise.resolve("attached");
			}
			if (command === "restore_agent_session") {
				return Promise.resolve("restored");
			}
			return Promise.reject(new Error(`unexpected command: ${command}`));
		});

		render(<AgentSessionRoute agentSessionId="agent-session-1" />);

		fireEvent.click(await screen.findByRole("button", { name: "Restore" }));

		expect(await screen.findByTestId("provider-terminal")).toBeVisible();
		expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
		expect(getReads).toBe(2);
	});

	it("同じworktreeの一覧変更後にbackend read modelを再取得する", async () => {
		const archived = {
			...session,
			lifecycle: "archived" as const,
			operations: {
				canArchive: false,
				canRestore: true,
				canDelete: true,
				canResume: false,
			},
		};
		let getReads = 0;
		let openCalls = 0;
		mockInvoke.mockImplementation((command) => {
			if (command === "get_agent_session") {
				getReads += 1;
				return Promise.resolve(getReads === 1 ? session : archived);
			}
			if (command === "open_agent_session") {
				openCalls += 1;
				return Promise.resolve(openCalls === 1 ? "attached" : "restored");
			}
			return Promise.reject(new Error(`unexpected command: ${command}`));
		});

		render(<AgentSessionRoute agentSessionId="agent-session-1" />);
		expect(await screen.findByTestId("provider-terminal")).toBeVisible();

		window.dispatchEvent(
			new CustomEvent("agent-session-refresh", {
				detail: { worktreePath: "/repo/worktree" },
			}),
		);

		expect(
			await screen.findByRole("button", { name: "Restore" }),
		).toBeVisible();
		expect(getReads).toBe(2);
		expect(openCalls).toBe(1);
	});
});
