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
			{ ptyId: 1, cols: 80 },
			{ ptyId: 2, cols: 120, label: "dev-server" },
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
				ptySessions={[{ ptyId: 1, cols: 80 }]}
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

	it("空リストで No terminal sessions と Start Terminal ボタンが表示される", () => {
		render(
			<TerminalTabContent
				{...defaultProps}
				ptySessions={[]}
				activeTab="terminal"
			/>,
		);
		expect(screen.getByText("No terminal sessions")).toBeInTheDocument();
		expect(screen.getByText("Start Terminal")).toBeInTheDocument();
	});
});
