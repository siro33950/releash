import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";

beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

describe("DeleteConfirmDialog", () => {
	const defaultProps = {
		open: true,
		itemName: "test-item",
		onConfirm: vi.fn(),
		onCancel: vi.fn(),
	};

	it("should display default description when description prop is not provided", () => {
		render(<DeleteConfirmDialog {...defaultProps} />);
		expect(
			screen.getByText('Delete "test-item"? This action cannot be undone.'),
		).toBeInTheDocument();
	});

	it("should display custom description when description prop is provided", () => {
		render(
			<DeleteConfirmDialog
				{...defaultProps}
				description="Remove from list? The repository will not be deleted from disk."
			/>,
		);
		expect(
			screen.getByText(
				"Remove from list? The repository will not be deleted from disk.",
			),
		).toBeInTheDocument();
	});

	it("should call onConfirm when Delete button is clicked", async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn();
		render(<DeleteConfirmDialog {...defaultProps} onConfirm={onConfirm} />);
		await user.click(screen.getByRole("button", { name: "Delete" }));
		expect(onConfirm).toHaveBeenCalledOnce();
	});

	it("should call onCancel when Cancel button is clicked", async () => {
		const user = userEvent.setup();
		const onCancel = vi.fn();
		render(<DeleteConfirmDialog {...defaultProps} onCancel={onCancel} />);
		await user.click(screen.getByRole("button", { name: "Cancel" }));
		expect(onCancel).toHaveBeenCalled();
	});
});
