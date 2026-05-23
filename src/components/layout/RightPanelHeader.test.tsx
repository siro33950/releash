import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PanelRight } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { RightPanelHeader } from "./RightPanelHeader";
import type { TogglePanel } from "./ViewToolbar";

function createPanels(overrides?: Partial<TogglePanel>): TogglePanel[] {
	return [
		{
			id: "right",
			icon: PanelRight,
			label: "Right Sidebar",
			visible: true,
			onToggle: vi.fn(),
			...overrides,
		},
	];
}

function renderHeader(panels: TogglePanel[]) {
	return render(
		<TooltipProvider>
			<RightPanelHeader panels={panels} />
		</TooltipProvider>,
	);
}

describe("RightPanelHeader", () => {
	it("renders toggle buttons for all panels", () => {
		const panels = createPanels();
		renderHeader(panels);

		expect(screen.getByLabelText("Toggle Right Sidebar")).toBeInTheDocument();
	});

	it("calls onToggle when a button is clicked", async () => {
		const user = userEvent.setup();
		const panels = createPanels();
		renderHeader(panels);

		await user.click(screen.getByLabelText("Toggle Right Sidebar"));
		expect(panels[0].onToggle).toHaveBeenCalledOnce();
	});

	it("applies foreground color to visible panels and muted to hidden", () => {
		const visiblePanels = createPanels({ visible: true });
		const { unmount } = renderHeader(visiblePanels);

		const visibleBtn = screen.getByLabelText("Toggle Right Sidebar");
		expect(visibleBtn).toHaveClass("text-foreground");
		expect(visibleBtn).not.toHaveClass("text-muted-foreground");

		unmount();

		const hiddenPanels = createPanels({ visible: false });
		renderHeader(hiddenPanels);

		const hiddenBtn = screen.getByLabelText("Toggle Right Sidebar");
		expect(hiddenBtn).toHaveClass("text-muted-foreground");
		expect(hiddenBtn).not.toHaveClass("text-foreground");
	});

	it("has data-tauri-drag-region attribute for window dragging", () => {
		const panels = createPanels();
		const { container } = renderHeader(panels);

		const header = container.querySelector("[data-tauri-drag-region]");
		expect(header).toBeInTheDocument();
	});

	// spec issues-1023: 右パネル上半分は Review 専用に戻り、表示モード切替は
	// 中央エリアの ViewToolbar に移動した。
	it("does not render any mode switch UI", () => {
		renderHeader(createPanels());
		expect(screen.queryByLabelText("Review mode")).toBeNull();
		expect(screen.queryByLabelText("Workflow mode")).toBeNull();
		expect(screen.queryByTestId("right-panel-mode-switch")).toBeNull();
	});
});
