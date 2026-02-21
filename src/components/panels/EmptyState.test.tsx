import { render, screen } from "@testing-library/react";
import { FolderOpen } from "lucide-react";
import { describe, expect, it } from "vitest";
import { EmptyState } from "./EmptyState";

describe("EmptyState", () => {
	it("should render title and default icon", () => {
		const { container } = render(<EmptyState title="No file selected" />);

		expect(screen.getByText("No file selected")).toBeInTheDocument();
		expect(container.querySelector("svg")).toBeInTheDocument();
	});

	it("should render description when provided", () => {
		render(
			<EmptyState
				title="No file selected"
				description="Select a file from the explorer"
			/>,
		);

		expect(screen.getByText("No file selected")).toBeInTheDocument();
		expect(
			screen.getByText("Select a file from the explorer"),
		).toBeInTheDocument();
	});

	it("should not render description when not provided", () => {
		const { container } = render(<EmptyState title="Empty" />);

		expect(container.querySelector("p")).toBeNull();
	});

	it("should render compact mode with text only", () => {
		const { container } = render(<EmptyState compact title="No changes" />);

		expect(screen.getByText("No changes")).toBeInTheDocument();
		// compact mode should not render h3 or svg icon
		expect(container.querySelector("h3")).toBeNull();
		expect(container.querySelector("svg")).toBeNull();
	});

	it("should apply custom className", () => {
		const { container } = render(
			<EmptyState title="Test" className="custom-class" />,
		);

		expect(container.firstChild).toHaveClass("custom-class");
	});

	it("should apply custom className in compact mode", () => {
		const { container } = render(
			<EmptyState compact title="Test" className="px-4 py-1.5" />,
		);

		expect(container.firstChild).toHaveClass("px-4");
	});

	it("should render custom icon", () => {
		const { container } = render(
			<EmptyState icon={FolderOpen} title="No folder" />,
		);

		expect(container.querySelector("svg")).toBeInTheDocument();
		expect(screen.getByText("No folder")).toBeInTheDocument();
	});

	it("should render children when provided", () => {
		render(
			<EmptyState title="Empty">
				<button type="button">Action</button>
			</EmptyState>,
		);

		expect(screen.getByRole("button", { name: "Action" })).toBeInTheDocument();
	});

	it("should render children in compact mode", () => {
		render(
			<EmptyState compact title="No results">
				<button type="button">Clear</button>
			</EmptyState>,
		);

		expect(screen.getByText("No results")).toBeInTheDocument();
		expect(screen.getByText("Clear")).toBeInTheDocument();
	});

	it("should render children in normal mode with description", () => {
		render(
			<EmptyState title="No results" description="Try adjusting filters">
				<button type="button">Reset</button>
			</EmptyState>,
		);

		expect(screen.getByText("No results")).toBeInTheDocument();
		expect(screen.getByText("Try adjusting filters")).toBeInTheDocument();
		expect(screen.getByText("Reset")).toBeInTheDocument();
	});

	it("should not render children when not provided", () => {
		render(<EmptyState compact title="Empty" />);

		expect(screen.getByText("Empty")).toBeInTheDocument();
		expect(screen.queryByRole("button")).toBeNull();
	});
});
