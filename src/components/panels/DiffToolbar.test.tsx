import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { DiffToolbarProps } from "./DiffToolbar";
import { DiffToolbar } from "./DiffToolbar";

vi.mock("@/contexts/EditorContext", () => ({
	useEditorContext: () => ({
		lspStatus: "idle" as const,
		lspError: null,
		lspCrashCount: 0,
		lspRetryManually: vi.fn(),
	}),
}));

// Radix UI Popover/Tooltip require pointer capture APIs not available in jsdom
if (typeof Element.prototype.hasPointerCapture !== "function") {
	Element.prototype.hasPointerCapture = () => false;
}
if (typeof Element.prototype.setPointerCapture !== "function") {
	Element.prototype.setPointerCapture = () => {};
}
if (typeof Element.prototype.releasePointerCapture !== "function") {
	Element.prototype.releasePointerCapture = () => {};
}

const defaultProps: DiffToolbarProps = {
	diffBase: "staged",
	diffMode: "inline",
	currentIndex: 0,
	total: 3,
	onDiffModeChange: vi.fn(),
	onGoToPrev: vi.fn(),
	onGoToNext: vi.fn(),
	onStageAll: vi.fn(),
	onUnstageAll: vi.fn(),
	showStageButtons: true,
};

function renderToolbar(props: Partial<DiffToolbarProps> = {}) {
	return render(
		<TooltipProvider>
			<DiffToolbar {...defaultProps} {...props} />
		</TooltipProvider>,
	);
}

describe("DiffToolbar", () => {
	describe("diffBase selector is not present in DiffToolbar", () => {
		it("should not render a diffBase select/combobox", () => {
			renderToolbar();

			expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
			expect(screen.queryByText("Staged")).not.toBeInTheDocument();
			expect(screen.queryByText("Branch Base")).not.toBeInTheDocument();
		});
	});

	describe("hunk navigation", () => {
		it("should render Previous and Next hunk buttons", () => {
			renderToolbar();

			expect(
				screen.getByRole("button", { name: "Previous hunk" }),
			).toBeInTheDocument();
			expect(
				screen.getByRole("button", { name: "Next hunk" }),
			).toBeInTheDocument();
		});

		it("should show current/total indicator", () => {
			renderToolbar({ currentIndex: 1, total: 5 });

			expect(screen.getByText("2/5")).toBeInTheDocument();
		});

		it("should hide navigation when total is 0", () => {
			renderToolbar({ total: 0 });

			expect(
				screen.queryByRole("button", { name: "Previous hunk" }),
			).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "Next hunk" }),
			).not.toBeInTheDocument();
		});
	});

	describe("diff mode toggle", () => {
		it("should render Gutter, Inline, and Split mode buttons", () => {
			renderToolbar();

			expect(
				screen.getByRole("button", { name: "Gutter" }),
			).toBeInTheDocument();
			expect(
				screen.getByRole("button", { name: "Inline" }),
			).toBeInTheDocument();
			expect(screen.getByRole("button", { name: "Split" })).toBeInTheDocument();
		});

		it("should mark current diff mode as pressed", () => {
			renderToolbar({ diffMode: "split" });

			expect(screen.getByRole("button", { name: "Split" })).toHaveAttribute(
				"aria-pressed",
				"true",
			);
			expect(screen.getByRole("button", { name: "Inline" })).toHaveAttribute(
				"aria-pressed",
				"false",
			);
		});
	});

	describe("stage buttons", () => {
		it("should show Stage All when showStageButtons is true and total > 0", () => {
			renderToolbar({ showStageButtons: true });

			expect(screen.getByText("Stage All")).toBeInTheDocument();
		});

		it("should hide Stage All when showStageButtons is false", () => {
			renderToolbar({ showStageButtons: false });

			expect(screen.queryByText("Stage All")).not.toBeInTheDocument();
		});

		it("should show Unstage All only when diffBase is branch-base", () => {
			renderToolbar({ diffBase: "branch-base", showStageButtons: true });

			expect(screen.getByText("Stage All")).toBeInTheDocument();
			expect(screen.getByText("Unstage All")).toBeInTheDocument();
		});

		it("should hide Unstage All when diffBase is staged", () => {
			renderToolbar({ diffBase: "staged", showStageButtons: true });

			expect(screen.getByText("Stage All")).toBeInTheDocument();
			expect(screen.queryByText("Unstage All")).not.toBeInTheDocument();
		});
	});
});
