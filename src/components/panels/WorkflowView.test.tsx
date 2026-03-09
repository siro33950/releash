import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkflowView } from "./WorkflowView";

vi.mock("react-resizable-panels", () => {
	const Panel = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel">{children}</div>
	);
	const Group = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel-group">{children}</div>
	);
	const Separator = () => <div data-testid="separator" />;
	return { Panel, Group, Separator };
});

vi.mock("@/components/panels/TerminalTabPanel", () => ({
	TerminalTabPanel: vi.fn(() => (
		<div data-testid="terminal-tab-panel">Terminal</div>
	)),
}));

describe("WorkflowView", () => {
	const defaultProps = {
		rootPath: "/repo",
		planDocument: "# Plan\nSome content",
		phase: "planning" as const,
		planTimeline: [],
		implTimeline: [],
		threads: [],
	};

	it("renders document viewer", () => {
		render(<WorkflowView {...defaultProps} />);

		expect(screen.getByTestId("workflow-document-viewer")).toBeInTheDocument();
	});

	it("renders terminal panel", () => {
		render(<WorkflowView {...defaultProps} />);

		expect(screen.getByTestId("terminal-tab-panel")).toBeInTheDocument();
	});

	it("renders plan and implementation panels", () => {
		render(<WorkflowView {...defaultProps} />);

		// Timeline tabs from both PlanPanel and ImplementationPanel
		const timelineTabs = screen.getAllByRole("tab", { name: "Timeline" });
		expect(timelineTabs.length).toBe(2);
	});

	it("shows implementation not started when phase is planning", () => {
		render(<WorkflowView {...defaultProps} phase="planning" />);

		expect(
			screen.getByText("Implementation has not started yet"),
		).toBeInTheDocument();
	});

	it("shows implementation timeline when phase is implementation", () => {
		render(
			<WorkflowView
				{...defaultProps}
				phase="implementation"
				implTimeline={[
					{
						id: "1",
						label: "Building",
						status: "in_progress",
						timestamp: Date.now(),
					},
				]}
			/>,
		);

		expect(screen.getByText("Building")).toBeInTheDocument();
	});

	it("accepts initialDocTerminalRatio and onDocTerminalResize props", () => {
		// Drag resize interaction is tested in Playwright integration tests.
		// Here we verify the props are accepted and rendering succeeds.
		const onDocTerminalResize = vi.fn();

		render(
			<WorkflowView
				{...defaultProps}
				initialDocTerminalRatio={[70, 30]}
				onDocTerminalResize={onDocTerminalResize}
			/>,
		);

		expect(screen.getByTestId("workflow-document-viewer")).toBeInTheDocument();
		expect(screen.getByTestId("terminal-tab-panel")).toBeInTheDocument();
	});

	it("renders center content independently of right panel props", () => {
		// rightPanelRef and onRightPanelResize are omitted
		render(<WorkflowView {...defaultProps} />);

		expect(screen.getByTestId("workflow-document-viewer")).toBeInTheDocument();
		expect(screen.getByTestId("terminal-tab-panel")).toBeInTheDocument();
		const timelineTabs = screen.getAllByRole("tab", { name: "Timeline" });
		expect(timelineTabs.length).toBe(2);
	});
});
