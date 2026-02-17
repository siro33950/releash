import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PanelBottom, PanelLeft, PanelRight } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { type TogglePanel, ViewToolbar } from "./ViewToolbar";

function createPanels(overrides?: Partial<TogglePanel>[]): TogglePanel[] {
	const defaults: TogglePanel[] = [
		{
			id: "sidebar",
			icon: PanelLeft,
			label: "Sidebar",
			visible: true,
			onToggle: vi.fn(),
		},
		{
			id: "review",
			icon: PanelBottom,
			label: "Review",
			visible: true,
			onToggle: vi.fn(),
		},
		{
			id: "terminal",
			icon: PanelRight,
			label: "Terminal",
			visible: false,
			onToggle: vi.fn(),
		},
	];
	if (overrides) {
		return defaults.map((d, i) => ({ ...d, ...overrides[i] }));
	}
	return defaults;
}

describe("ViewToolbar", () => {
	it("renders toggle buttons for all panels", () => {
		const panels = createPanels();
		render(<ViewToolbar panels={panels} />);

		expect(screen.getByLabelText("Toggle Sidebar")).toBeInTheDocument();
		expect(screen.getByLabelText("Toggle Review")).toBeInTheDocument();
		expect(screen.getByLabelText("Toggle Terminal")).toBeInTheDocument();
	});

	it("calls onToggle when a button is clicked", async () => {
		const user = userEvent.setup();
		const panels = createPanels();
		render(<ViewToolbar panels={panels} />);

		await user.click(screen.getByLabelText("Toggle Sidebar"));
		expect(panels[0].onToggle).toHaveBeenCalledOnce();

		await user.click(screen.getByLabelText("Toggle Terminal"));
		expect(panels[2].onToggle).toHaveBeenCalledOnce();
	});

	it("applies foreground color to visible panels and muted to hidden", () => {
		const panels = createPanels();
		render(<ViewToolbar panels={panels} />);

		const sidebarBtn = screen.getByLabelText("Toggle Sidebar");
		const terminalBtn = screen.getByLabelText("Toggle Terminal");

		expect(sidebarBtn.className).toContain("text-foreground");
		expect(terminalBtn.className).toContain("text-muted-foreground");
	});
});
