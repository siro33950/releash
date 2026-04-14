import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import { ScrollArea } from "./scroll-area";

describe("ScrollArea", () => {
	it("renders children", () => {
		render(
			<ScrollArea>
				<div>Content</div>
			</ScrollArea>,
		);
		expect(screen.getByText("Content")).toBeInTheDocument();
	});

	it("forwards viewportRef to viewport element", () => {
		const ref = createRef<HTMLDivElement>();
		const { container } = render(
			<ScrollArea viewportRef={ref}>
				<div>Content</div>
			</ScrollArea>,
		);
		const viewport = container.querySelector(
			'[data-slot="scroll-area-viewport"]',
		);
		expect(ref.current).toBe(viewport);
	});

	it("forwards onScroll to viewport element", () => {
		const onScroll = vi.fn();
		const { container } = render(
			<ScrollArea onScroll={onScroll}>
				<div>Content</div>
			</ScrollArea>,
		);
		const viewport = container.querySelector(
			'[data-slot="scroll-area-viewport"]',
		);
		expect(viewport).not.toBeNull();
		fireEvent.scroll(viewport!);
		expect(onScroll).toHaveBeenCalledTimes(1);
	});

	it("works without viewportRef and onScroll", () => {
		const { container } = render(
			<ScrollArea>
				<div>Content</div>
			</ScrollArea>,
		);
		const viewport = container.querySelector(
			'[data-slot="scroll-area-viewport"]',
		);
		expect(viewport).toBeInTheDocument();
		expect(screen.getByText("Content")).toBeInTheDocument();
	});
});
