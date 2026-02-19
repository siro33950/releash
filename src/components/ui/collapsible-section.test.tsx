import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CollapsibleSection } from "./collapsible-section";

describe("CollapsibleSection", () => {
	it("renders children when defaultOpen is true", () => {
		render(
			<CollapsibleSection title="Section">
				<div>Content</div>
			</CollapsibleSection>,
		);
		expect(screen.getByText("Content")).toBeInTheDocument();
	});

	it("hides children when defaultOpen is false", () => {
		render(
			<CollapsibleSection title="Section" defaultOpen={false}>
				<div>Content</div>
			</CollapsibleSection>,
		);
		expect(screen.queryByText("Content")).not.toBeInTheDocument();
	});

	it("toggles children visibility on header click", () => {
		render(
			<CollapsibleSection title="Section">
				<div>Content</div>
			</CollapsibleSection>,
		);
		expect(screen.getByText("Content")).toBeInTheDocument();

		fireEvent.click(screen.getByRole("button", { name: /Section/ }));
		expect(screen.queryByText("Content")).not.toBeInTheDocument();

		fireEvent.click(screen.getByRole("button", { name: /Section/ }));
		expect(screen.getByText("Content")).toBeInTheDocument();
	});

	it("displays count in title", () => {
		render(
			<CollapsibleSection title="Items" count={5}>
				<div>Content</div>
			</CollapsibleSection>,
		);
		expect(screen.getByText("Items (5)")).toBeInTheDocument();
	});

	it("does not display count when not provided", () => {
		render(
			<CollapsibleSection title="Items">
				<div>Content</div>
			</CollapsibleSection>,
		);
		expect(screen.getByText("Items")).toBeInTheDocument();
	});

	it("renders actions slot", () => {
		render(
			<CollapsibleSection
				title="Section"
				actions={<button type="button">Action</button>}
			>
				<div>Content</div>
			</CollapsibleSection>,
		);
		expect(screen.getByRole("button", { name: "Action" })).toBeInTheDocument();
	});

	it("applies className to root", () => {
		const { container } = render(
			<CollapsibleSection title="Section" className="custom-root">
				<div>Content</div>
			</CollapsibleSection>,
		);
		const root = container.querySelector('[data-slot="collapsible-section"]');
		expect(root).toHaveClass("custom-root");
	});

	it("applies headerClassName to header", () => {
		const { container } = render(
			<CollapsibleSection title="Section" headerClassName="custom-header">
				<div>Content</div>
			</CollapsibleSection>,
		);
		const header = container.querySelector(
			'[data-slot="collapsible-section"] > div',
		);
		expect(header).toHaveClass("custom-header");
	});

	it("applies chevronClassName to chevron icon", () => {
		const { container } = render(
			<CollapsibleSection title="Section" chevronClassName="custom-chevron">
				<div>Content</div>
			</CollapsibleSection>,
		);
		const svg = container.querySelector(
			'[data-slot="collapsible-section"] button svg',
		);
		expect(svg).toHaveClass("custom-chevron");
	});
});
