import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@xyflow/react", () => ({
	Handle: () => <div />,
	Position: { Top: "top", Bottom: "bottom" },
}));

const { StepNode } = await import("./StepNode");
type StepNodeData = import("./StepNode").StepNodeData;

function makeNodeProps(data: Partial<StepNodeData> = {}) {
	const nodeData: StepNodeData = {
		label: "test-step",
		mode: "auto",
		state: "pending",
		executionCount: 0,
		isCurrent: false,
		...data,
	};
	return { id: "test-node", data: nodeData } as unknown as Parameters<
		typeof StepNode
	>[0];
}

describe("StepNode", () => {
	it("renders step label", () => {
		render(<StepNode {...makeNodeProps({ label: "my-step" })} />);
		expect(screen.getByText("my-step")).toBeInTheDocument();
	});

	it("renders step mode", () => {
		render(<StepNode {...makeNodeProps({ mode: "approval" })} />);
		expect(screen.getByText("approval")).toBeInTheDocument();
	});

	it("applies running color class", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ state: "running" })} />,
		);
		expect(container.querySelector(".border-blue-500")).toBeInTheDocument();
	});

	it("applies completed color class", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ state: "completed" })} />,
		);
		expect(container.querySelector(".border-green-500")).toBeInTheDocument();
	});

	it("applies failed color class", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ state: "failed" })} />,
		);
		expect(container.querySelector(".border-red-500")).toBeInTheDocument();
	});

	it("applies waiting_approval color class", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ state: "waiting_approval" })} />,
		);
		expect(container.querySelector(".border-yellow-500")).toBeInTheDocument();
	});

	it("applies pending color class", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ state: "pending" })} />,
		);
		expect(
			container.querySelector(".border-muted-foreground\\/30"),
		).toBeInTheDocument();
	});

	it("shows ring indicator when isCurrent is true", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ isCurrent: true })} />,
		);
		expect(container.querySelector(".ring-2")).toBeInTheDocument();
	});

	it("does not show ring indicator when isCurrent is false", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ isCurrent: false })} />,
		);
		expect(container.querySelector(".ring-2")).not.toBeInTheDocument();
	});

	it("shows execution count badge when executionCount > 0", () => {
		render(<StepNode {...makeNodeProps({ executionCount: 3 })} />);
		expect(screen.getByText("×3")).toBeInTheDocument();
	});

	it("does not show execution count badge when executionCount is 0", () => {
		render(<StepNode {...makeNodeProps({ executionCount: 0 })} />);
		expect(screen.queryByText(/×/)).not.toBeInTheDocument();
	});

	it("applies aborted color class", () => {
		const { container } = render(
			<StepNode {...makeNodeProps({ state: "aborted" })} />,
		);
		expect(
			container.querySelector(".border-muted-foreground"),
		).toBeInTheDocument();
	});
});
