import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { InlineMessage } from "./inline-message";

describe("InlineMessage", () => {
	it("renders children", () => {
		render(<InlineMessage>Something went wrong</InlineMessage>);
		expect(screen.getByText("Something went wrong")).toBeInTheDocument();
	});

	it("applies error type by default", () => {
		const { container } = render(<InlineMessage>Error</InlineMessage>);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-destructive");
	});

	it("applies success type", () => {
		const { container } = render(
			<InlineMessage type="success">Done</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-success");
	});

	it("applies warning type", () => {
		const { container } = render(
			<InlineMessage type="warning">Caution</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-warning");
	});

	it("applies info type", () => {
		const { container } = render(
			<InlineMessage type="info">Info</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-info");
	});

	it("applies xs size", () => {
		const { container } = render(<InlineMessage size="xs">Tiny</InlineMessage>);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-[10px]");
	});

	it("applies sm size by default", () => {
		const { container } = render(<InlineMessage>Small</InlineMessage>);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-xs");
	});

	it("applies default size", () => {
		const { container } = render(
			<InlineMessage size="default">Normal</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("text-sm");
	});

	it("applies filled variant", () => {
		const { container } = render(<InlineMessage filled>Error</InlineMessage>);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("bg-destructive/10");
		expect(el).toHaveClass("rounded");
		expect(el).toHaveClass("px-2");
	});

	it("applies filled warning with border", () => {
		const { container } = render(
			<InlineMessage type="warning" filled>
				Warning
			</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("bg-warning/10");
	});

	it("does not show icon by default", () => {
		const { container } = render(<InlineMessage>Error</InlineMessage>);
		expect(container.querySelector("svg")).toBeNull();
	});

	it("shows icon when icon prop is true", () => {
		const { container } = render(<InlineMessage icon>Error</InlineMessage>);
		expect(container.querySelector("svg")).not.toBeNull();
	});

	it("shows dismiss button when onDismiss is provided", () => {
		const onDismiss = vi.fn();
		render(<InlineMessage onDismiss={onDismiss}>Error</InlineMessage>);
		const buttons = screen.getAllByRole("button");
		expect(buttons).toHaveLength(1);
		fireEvent.click(buttons[0]);
		expect(onDismiss).toHaveBeenCalledOnce();
	});

	it("shows retry button when onRetry is provided (inline)", () => {
		const onRetry = vi.fn();
		render(<InlineMessage onRetry={onRetry}>Error</InlineMessage>);
		const retryBtn = screen.getByText("Retry");
		fireEvent.click(retryBtn);
		expect(onRetry).toHaveBeenCalledOnce();
	});

	it("renders block layout with retry", () => {
		const onRetry = vi.fn();
		const { container } = render(
			<InlineMessage layout="block" onRetry={onRetry}>
				Failed
			</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("flex-col");
		expect(el).toHaveClass("items-center");
		const retryBtn = screen.getByText("Retry");
		fireEvent.click(retryBtn);
		expect(onRetry).toHaveBeenCalledOnce();
	});

	it("merges className", () => {
		const { container } = render(
			<InlineMessage className="mt-2 px-3">Error</InlineMessage>,
		);
		const el = container.querySelector('[data-slot="inline-message"]');
		expect(el).toHaveClass("mt-2");
		expect(el).toHaveClass("px-3");
	});

	it("passes through HTML attributes", () => {
		render(<InlineMessage data-testid="msg">Error</InlineMessage>);
		expect(screen.getByTestId("msg")).toBeInTheDocument();
	});
});
