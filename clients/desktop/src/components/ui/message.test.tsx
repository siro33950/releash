import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Message } from "./message";

describe("Message", () => {
	it("renders inline error message by default", () => {
		render(<Message message="Something went wrong" />);
		expect(screen.getByText("Something went wrong")).toBeInTheDocument();
		const root = screen
			.getByText("Something went wrong")
			.closest('[data-slot="message"]');
		expect(root).toHaveClass("text-destructive");
		expect(root).toHaveClass("text-xs");
	});

	it("renders block variant with icon", () => {
		render(<Message variant="block" message="Failed to load" />);
		const root = screen
			.getByText("Failed to load")
			.closest('[data-slot="message"]');
		expect(root).toHaveClass("flex-col", "items-center", "text-sm");
	});

	it("applies severity classes", () => {
		const { rerender } = render(<Message severity="warning" message="warn" />);
		let root = screen.getByText("warn").closest('[data-slot="message"]');
		expect(root).toHaveClass("text-warning");

		rerender(<Message severity="info" message="info msg" />);
		root = screen.getByText("info msg").closest('[data-slot="message"]');
		expect(root).toHaveClass("text-info");

		rerender(<Message severity="success" message="ok" />);
		root = screen.getByText("ok").closest('[data-slot="message"]');
		expect(root).toHaveClass("text-success");
	});

	it("applies xs size", () => {
		render(<Message size="xs" message="small" />);
		const root = screen.getByText("small").closest('[data-slot="message"]');
		expect(root).toHaveClass("text-[10px]");
	});

	it("shows dismiss button when onDismiss provided", () => {
		const onDismiss = vi.fn();
		render(<Message message="err" onDismiss={onDismiss} />);
		const buttons = screen.getAllByRole("button");
		expect(buttons).toHaveLength(1);
		fireEvent.click(buttons[0]);
		expect(onDismiss).toHaveBeenCalledOnce();
	});

	it("does not show dismiss button when onDismiss not provided", () => {
		render(<Message message="err" />);
		expect(screen.queryAllByRole("button")).toHaveLength(0);
	});

	it("shows retry button in block variant", () => {
		const onRetry = vi.fn();
		render(<Message variant="block" message="Failed" onRetry={onRetry} />);
		const retryBtn = screen.getByText("Retry");
		fireEvent.click(retryBtn);
		expect(onRetry).toHaveBeenCalledOnce();
	});

	it("does not show retry button when onRetry not provided", () => {
		render(<Message variant="block" message="Failed" />);
		expect(screen.queryByText("Retry")).not.toBeInTheDocument();
	});

	it("toggles expand/collapse when expandable", () => {
		render(<Message message="long error text here" expandable />);
		const msgSpan = screen.getByText("long error text here");
		expect(msgSpan).toHaveClass("truncate");

		const toggleBtn = screen.getAllByRole("button")[0];
		fireEvent.click(toggleBtn);
		expect(msgSpan).not.toHaveClass("truncate");

		fireEvent.click(toggleBtn);
		expect(msgSpan).toHaveClass("truncate");
	});

	it("applies custom className", () => {
		render(<Message message="test" className="bg-red-500 px-2" />);
		const root = screen.getByText("test").closest('[data-slot="message"]');
		expect(root).toHaveClass("bg-red-500", "px-2");
	});
});
