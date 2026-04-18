import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { DiffToolbarProps } from "./DiffToolbar";
import { DiffToolbar } from "./DiffToolbar";

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
	diffMode: "inline",
	currentIndex: 0,
	total: 3,
	onDiffModeChange: vi.fn(),
	onGoToPrev: vi.fn(),
	onGoToNext: vi.fn(),
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

	describe("stage buttons removed from toolbar", () => {
		it("should not show Stage All or Unstage All buttons", () => {
			renderToolbar();

			expect(screen.queryByText("Stage All")).not.toBeInTheDocument();
			expect(screen.queryByText("Unstage All")).not.toBeInTheDocument();
		});
	});
});
