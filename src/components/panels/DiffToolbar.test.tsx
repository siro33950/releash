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
	onDiffModeChange: vi.fn(),
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

	describe("file navigation", () => {
		it("should render Previous/Next file buttons when fileNavigation is provided", () => {
			renderToolbar({
				fileNavigation: {
					current_index: 1,
					total: 3,
					prev_file: "a.ts",
					next_file: "c.ts",
				},
				onGoToPrevFile: vi.fn(),
				onGoToNextFile: vi.fn(),
			});

			expect(
				screen.getByRole("button", { name: "Previous file" }),
			).toBeInTheDocument();
			expect(
				screen.getByRole("button", { name: "Next file" }),
			).toBeInTheDocument();
			expect(screen.getByText("2/3")).toBeInTheDocument();
		});

		it("should disable Previous file when prev_file is null", () => {
			renderToolbar({
				fileNavigation: {
					current_index: 0,
					total: 3,
					prev_file: null,
					next_file: "b.ts",
				},
				onGoToPrevFile: vi.fn(),
				onGoToNextFile: vi.fn(),
			});

			expect(
				screen.getByRole("button", { name: "Previous file" }),
			).toBeDisabled();
			expect(
				screen.getByRole("button", { name: "Next file" }),
			).not.toBeDisabled();
		});

		it("should disable Next file when next_file is null", () => {
			renderToolbar({
				fileNavigation: {
					current_index: 2,
					total: 3,
					prev_file: "b.ts",
					next_file: null,
				},
				onGoToPrevFile: vi.fn(),
				onGoToNextFile: vi.fn(),
			});

			expect(
				screen.getByRole("button", { name: "Previous file" }),
			).not.toBeDisabled();
			expect(screen.getByRole("button", { name: "Next file" })).toBeDisabled();
		});

		it("should disable both buttons when total is 1", () => {
			renderToolbar({
				fileNavigation: {
					current_index: 0,
					total: 1,
					prev_file: null,
					next_file: null,
				},
				onGoToPrevFile: vi.fn(),
				onGoToNextFile: vi.fn(),
			});

			expect(
				screen.getByRole("button", { name: "Previous file" }),
			).toBeDisabled();
			expect(screen.getByRole("button", { name: "Next file" })).toBeDisabled();
		});

		it("should not render file navigation when fileNavigation is not provided", () => {
			renderToolbar();

			expect(
				screen.queryByRole("button", { name: "Previous file" }),
			).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "Next file" }),
			).not.toBeInTheDocument();
		});
	});
});
