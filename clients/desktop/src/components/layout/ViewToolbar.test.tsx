import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PanelLeft } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { type TogglePanel, ViewToolbar } from "./ViewToolbar";

function renderToolbar(props: {
	leftPanels?: TogglePanel[];
	rightSlot?: React.ReactNode;
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
});
