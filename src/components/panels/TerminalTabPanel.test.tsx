import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalTabPanel } from "./TerminalTabPanel";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: vi.fn(() => <div data-testid="terminal-panel" />),
}));

describe("TerminalTabPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("初回レンダリングで Terminal 1 タブが表示される", () => {
		render(<TerminalTabPanel />);
		expect(screen.getByText("Terminal 1")).toBeInTheDocument();
	});

	it("+ ボタンでタブが追加される", async () => {
		const user = userEvent.setup();
		render(<TerminalTabPanel />);

		const addButton = screen.getByLabelText("Add terminal tab");
		await user.click(addButton);

		expect(screen.getByText("Terminal 2")).toBeInTheDocument();
	});

	it("タブ切替が動作する", async () => {
		const user = userEvent.setup();
		render(<TerminalTabPanel />);

		const addButton = screen.getByLabelText("Add terminal tab");
		await user.click(addButton);

		const tab1 = screen.getByText("Terminal 1");
		const tab2 = screen.getByText("Terminal 2");

		await user.click(tab1);
		expect(tab1.closest("[role='tab']")).toHaveAttribute(
			"aria-selected",
			"true",
		);

		await user.click(tab2);
		expect(tab2.closest("[role='tab']")).toHaveAttribute(
			"aria-selected",
			"true",
		);
	});

	it("x ボタンでタブが閉じられる", async () => {
		const user = userEvent.setup();
		render(<TerminalTabPanel />);

		const addButton = screen.getByLabelText("Add terminal tab");
		await user.click(addButton);

		expect(screen.getByText("Terminal 2")).toBeInTheDocument();

		const closeButton = screen.getByLabelText("Close Terminal 2");
		await user.click(closeButton);

		expect(screen.queryByText("Terminal 2")).not.toBeInTheDocument();
	});

	it("最後のタブは閉じられない", () => {
		render(<TerminalTabPanel />);
		expect(screen.queryByLabelText("Close Terminal 1")).not.toBeInTheDocument();
	});

	it("StrictMode: 削除後の追加で連番が崩れない", async () => {
		const user = userEvent.setup();
		render(
			<StrictMode>
				<TerminalTabPanel />
			</StrictMode>,
		);

		const addButton = screen.getByLabelText("Add terminal tab");
		await user.click(addButton);
		expect(screen.getByText("Terminal 1")).toBeInTheDocument();
		expect(screen.getByText("Terminal 2")).toBeInTheDocument();

		await user.click(screen.getByLabelText("Close Terminal 1"));

		await user.click(addButton);
		const tabLabels = screen
			.getAllByRole("tab")
			.map((el) => el.textContent?.replace(/×$/, "").trim());
		expect(tabLabels).toEqual(["Terminal 2", "Terminal 3"]);
	});

	it("最大8タブまで追加可能", async () => {
		const user = userEvent.setup();
		render(<TerminalTabPanel />);

		const addButton = screen.getByLabelText("Add terminal tab");
		for (let i = 0; i < 7; i++) {
			await user.click(addButton);
		}

		expect(screen.getAllByRole("tab")).toHaveLength(8);
		expect(screen.queryByLabelText("Add terminal tab")).not.toBeInTheDocument();
	});
});
