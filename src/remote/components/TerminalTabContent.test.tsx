import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalTabContent } from "./TerminalTabContent";

vi.mock("./RemoteTerminalPanel", () => ({
	RemoteTerminalPanel: vi.fn(() => <div data-testid="remote-terminal-panel" />),
}));

describe("TerminalTabContent", () => {
	const defaultProps = {
		status: "connected" as const,
		ptySessions: [
			{ ptyId: 1, cols: 80, kind: "terminal" as const },
			{ ptyId: 2, cols: 120, label: "dev-server", kind: "terminal" as const },
		],
		activePtyId: 1,
		ptySpawning: false,
		ptySpawnError: null,
		terminalMounted: true,
		selectedWorktree: "/repo",
		activeTab: "terminal",
		send: vi.fn(),
		subscribe: vi.fn(() => () => {}),
		setActivePtyId: vi.fn(),
		spawnPty: vi.fn(),
		killPty: vi.fn(),
	};

	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("label がある場合は label を表示する", () => {
		render(<TerminalTabContent {...defaultProps} />);
		expect(screen.getByText("dev-server")).toBeInTheDocument();
	});

	it("label がない場合は Terminal N を表示する", () => {
		render(<TerminalTabContent {...defaultProps} />);
		expect(screen.getByText("Terminal 1")).toBeInTheDocument();
	});

	it("タブバーが常に表示される（1セッション時も）", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[{ ptyId: 1, cols: 80, kind: "terminal" as const }]}
			/>,
		);
		expect(screen.getByRole("tab")).toBeInTheDocument();
		expect(screen.getByLabelText("Add terminal")).toBeInTheDocument();
	});

	it("+ ボタンで spawnPty が呼ばれる", async () => {
		const user = userEvent.setup();
		render(<TerminalTabContent {...defaultProps} />);

		const addButton = screen.getByLabelText("Add terminal");
		await user.click(addButton);

		expect(defaultProps.spawnPty).toHaveBeenCalledOnce();
	});

	it("x ボタンで killPty が呼ばれる", async () => {
		const user = userEvent.setup();
		render(<TerminalTabContent {...defaultProps} />);

		const closeButton = screen.getByLabelText("Close Terminal 1");
		await user.click(closeButton);

		expect(defaultProps.killPty).toHaveBeenCalledWith(1);
	});

	it("label 付きタブの x ボタンで killPty が呼ばれる", async () => {
		const user = userEvent.setup();
		render(<TerminalTabContent {...defaultProps} />);

		const closeButton = screen.getByLabelText("Close dev-server");
		await user.click(closeButton);

		expect(defaultProps.killPty).toHaveBeenCalledWith(2);
	});

	it("タブクリックで setActivePtyId が呼ばれる", async () => {
		const user = userEvent.setup();
		render(<TerminalTabContent {...defaultProps} />);

		const tab2 = screen.getByText("dev-server");
		await user.click(tab2);

		expect(defaultProps.setActivePtyId).toHaveBeenCalledWith(2);
	});

	it("agent mode 時に + ボタンが非表示", () => {
		render(
			<TerminalTabContent {...defaultProps} mode="agent" activeTab="agent" />,
		);
		expect(screen.queryByLabelText("Add terminal")).not.toBeInTheDocument();
	});

	it("agent mode + 空リストで Spawn ボタン非表示", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[]}
				mode="agent"
				activeTab="agent"
			/>,
		);
		expect(screen.queryByText("Start Terminal")).not.toBeInTheDocument();
		expect(screen.getByText("No agent sessions")).toBeInTheDocument();
	});

	it("agent mode でラベル未設定時に Agent N と表示される", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[
					{ ptyId: 1, cols: 80, kind: "agent" as const },
					{ ptyId: 2, cols: 120, label: "my-agent", kind: "agent" as const },
				]}
				mode="agent"
				activeTab="agent"
			/>,
		);
		expect(screen.getByText("Agent 1")).toBeInTheDocument();
		expect(screen.getByText("my-agent")).toBeInTheDocument();
		expect(screen.getByLabelText("Close Agent 1")).toBeInTheDocument();
		expect(screen.getByLabelText("Close my-agent")).toBeInTheDocument();
		expect(screen.getByRole("tablist")).toHaveAttribute("aria-label", "Agent Tabs");
	});

	it("agentStates が渡された場合に AgentStateIcon が表示される", () => {
		const agentStates = new Map([
			[
				"/repo::1",
				{
					worktree_path: "/repo",
					state: "running" as const,
					exit_code: null,
					timestamp: Date.now(),
					session_id: null,
					pty_id: "1",
				},
			],
		]);
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[
					{ ptyId: 1, cols: 80, worktreePath: "/repo", kind: "agent" as const },
				]}
				agentStates={agentStates}
				mode="agent"
				activeTab="agent"
			/>,
		);
		expect(screen.getByTitle("running")).toBeInTheDocument();
	});

	it("agentStates が渡されていない場合は AgentStateIcon が表示されない", () => {
		render(<TerminalTabContent {...defaultProps} />);
		expect(screen.queryByTitle("running")).not.toBeInTheDocument();
	});

	it("terminal mode で Terminal セッションのみ表示され Agent セッションが混入しない", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[
					{ ptyId: 1, cols: 80, kind: "terminal" as const },
					{ ptyId: 2, cols: 80, kind: "terminal" as const, label: "dev" },
				]}
				mode="terminal"
				activeTab="terminal"
			/>,
		);
		expect(screen.getByText("Terminal 1")).toBeInTheDocument();
		expect(screen.getByText("dev")).toBeInTheDocument();
		expect(screen.queryByText(/Agent/)).not.toBeInTheDocument();
	});

	it("agent mode で Agent セッションのみ表示され Terminal セッションが混入しない", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[
					{ ptyId: 3, cols: 80, kind: "agent" as const },
					{ ptyId: 4, cols: 80, kind: "agent" as const, label: "claude" },
				]}
				mode="agent"
				activeTab="agent"
			/>,
		);
		expect(screen.getByText("Agent 3")).toBeInTheDocument();
		expect(screen.getByText("claude")).toBeInTheDocument();
		expect(screen.queryByText(/Terminal \d/)).not.toBeInTheDocument();
	});

	it("terminal mode + 空リストで No terminal sessions と Start Terminal ボタンが表示される", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[]}
				mode="terminal"
				activeTab="terminal"
			/>,
		);
		expect(screen.getByText("No terminal sessions")).toBeInTheDocument();
		expect(screen.getByText("Start Terminal")).toBeInTheDocument();
	});
});
