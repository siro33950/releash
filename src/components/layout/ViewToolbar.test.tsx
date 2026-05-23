import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PanelLeft } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { type CenterMode, type TogglePanel, ViewToolbar } from "./ViewToolbar";

function renderToolbar(props: {
	leftPanels?: TogglePanel[];
	rightSlot?: React.ReactNode;
	mode?: CenterMode;
	onModeChange?: (mode: CenterMode) => void;
}) {
	return render(
		<TooltipProvider>
			<ViewToolbar {...props} />
		</TooltipProvider>,
	);
}

describe("ViewToolbar", () => {
	it("has data-tauri-drag-region attribute for window dragging", () => {
		const { container } = renderToolbar({});

		const toolbar = container.querySelector("[data-tauri-drag-region]");
		expect(toolbar).toBeInTheDocument();
	});

	it("renders leftPanels toggle buttons when hidden", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		const leftPanels: TogglePanel[] = [
			{
				id: "left-nav",
				icon: PanelLeft,
				label: "Sidebar",
				visible: false,
				onToggle,
			},
		];

		renderToolbar({ leftPanels });

		const btn = screen.getByLabelText("Toggle Sidebar");
		expect(btn).toBeInTheDocument();
		expect(btn).toHaveClass("text-muted-foreground");

		await user.click(btn);
		expect(onToggle).toHaveBeenCalledOnce();
	});

	it("renders leftPanels toggle button with foreground when visible", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		const leftPanels: TogglePanel[] = [
			{
				id: "left-nav",
				icon: PanelLeft,
				label: "Sidebar",
				visible: true,
				onToggle,
			},
		];

		renderToolbar({ leftPanels });

		const btn = screen.getByLabelText("Toggle Sidebar");
		expect(btn).toHaveClass("text-foreground");
		expect(btn).not.toHaveClass("text-muted-foreground");

		await user.click(btn);
		expect(onToggle).toHaveBeenCalledOnce();
	});

	it("renders rightSlot content", () => {
		renderToolbar({
			rightSlot: <span data-testid="branch">main</span>,
		});

		expect(screen.getByTestId("branch")).toBeInTheDocument();
	});

	// spec issues-1023: 中央エリアの AgentChat / Workflow 切替は ViewToolbar 上の
	// セグメントコントロールで操作する。
	it("renders center mode switch when mode and onModeChange are provided", () => {
		renderToolbar({ mode: "agent", onModeChange: vi.fn() });

		expect(screen.getByTestId("center-mode-switch")).toBeInTheDocument();
		expect(screen.getByLabelText("Agent mode")).toHaveAttribute(
			"aria-pressed",
			"true",
		);
		expect(screen.getByLabelText("Workflow mode")).toHaveAttribute(
			"aria-pressed",
			"false",
		);
	});

	it("calls onModeChange when switching to Workflow", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		renderToolbar({ mode: "agent", onModeChange });

		await user.click(screen.getByLabelText("Workflow mode"));
		expect(onModeChange).toHaveBeenCalledWith("workflow");
	});

	it("does not render center mode switch when mode props are omitted", () => {
		renderToolbar({});
		expect(screen.queryByTestId("center-mode-switch")).toBeNull();
		expect(screen.queryByLabelText("Agent mode")).toBeNull();
		expect(screen.queryByLabelText("Workflow mode")).toBeNull();
	});
});
