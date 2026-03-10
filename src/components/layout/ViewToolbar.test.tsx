import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PanelLeft, PanelRight } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { Tabs } from "@/components/ui/tabs";
import { TooltipProvider } from "@/components/ui/tooltip";
import { type TogglePanel, ViewToolbar } from "./ViewToolbar";

function renderToolbar(
	props: {
		leftPanels?: TogglePanel[];
		rightPanels?: TogglePanel[];
		rightSlot?: React.ReactNode;
	},
	tabsProps?: { value?: string; onValueChange?: (v: string) => void },
) {
	return render(
		<TooltipProvider>
			<Tabs
				value={tabsProps?.value ?? "editor"}
				onValueChange={tabsProps?.onValueChange}
			>
				<ViewToolbar {...props} />
			</Tabs>
		</TooltipProvider>,
	);
}

describe("ViewToolbar", () => {
	it("renders TabsList with Workflow and Editor triggers", () => {
		renderToolbar({});

		expect(screen.getByRole("tab", { name: "Workflow" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Editor" })).toBeInTheDocument();
	});

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

	it("renders rightPanels toggle buttons", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		const rightPanels: TogglePanel[] = [
			{
				id: "right",
				icon: PanelRight,
				label: "Right Sidebar",
				visible: true,
				onToggle,
			},
		];

		renderToolbar({ rightPanels });

		const btn = screen.getByLabelText("Toggle Right Sidebar");
		expect(btn).toBeInTheDocument();
		expect(btn).toHaveClass("text-foreground");

		await user.click(btn);
		expect(onToggle).toHaveBeenCalledOnce();
	});

	it("renders rightPanels with muted color when not visible", () => {
		const rightPanels: TogglePanel[] = [
			{
				id: "right",
				icon: PanelRight,
				label: "Right Sidebar",
				visible: false,
				onToggle: vi.fn(),
			},
		];

		renderToolbar({ rightPanels });

		const btn = screen.getByLabelText("Toggle Right Sidebar");
		expect(btn).toHaveClass("text-muted-foreground");
		expect(btn).not.toHaveClass("text-foreground");
	});

	it("calls onValueChange with 'workflow' when Workflow tab is clicked", async () => {
		const user = userEvent.setup();
		const onValueChange = vi.fn();

		renderToolbar({}, { value: "editor", onValueChange });

		await user.click(screen.getByRole("tab", { name: "Workflow" }));
		expect(onValueChange).toHaveBeenCalledWith("workflow");
	});

	it("calls onValueChange with 'editor' when Editor tab is clicked", async () => {
		const user = userEvent.setup();
		const onValueChange = vi.fn();

		renderToolbar({}, { value: "workflow", onValueChange });

		await user.click(screen.getByRole("tab", { name: "Editor" }));
		expect(onValueChange).toHaveBeenCalledWith("editor");
	});

	it("Workflow tab is selected by default when value is 'workflow'", () => {
		renderToolbar({}, { value: "workflow" });
		const workflowTab = screen.getByRole("tab", { name: "Workflow" });
		expect(workflowTab).toHaveAttribute("aria-selected", "true");

		const editorTab = screen.getByRole("tab", { name: "Editor" });
		expect(editorTab).toHaveAttribute("aria-selected", "false");
	});

	it("does not render Agent tab", () => {
		renderToolbar({});
		expect(
			screen.queryByRole("tab", { name: "Agent" }),
		).not.toBeInTheDocument();
	});
});
