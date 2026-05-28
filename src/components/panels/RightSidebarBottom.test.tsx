import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RightSidebarBottom } from "./RightSidebarBottom";

// react-resizable-panels does not work in jsdom
vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => (
		<div data-testid="resizable-group">{children}</div>
	),
	Panel: ({
		children,
		id,
	}: {
		children: React.ReactNode;
		id?: string;
		[key: string]: unknown;
	}) => <div data-testid={`panel-${id ?? "unknown"}`}>{children}</div>,
	Separator: () => <div data-testid="separator" />,
}));

vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: () => <div data-testid="terminal-panel" />,
}));

vi.mock("@/hooks/useDiffComments", () => ({
	useDiffComments: () => ({
		comments: [],
		loading: false,
		unsentCount: 0,
		addComment: vi.fn(),
		appendComment: vi.fn(),
		resolveThread: vi.fn(),
		deleteThread: vi.fn().mockResolvedValue(undefined),
		getCommentsForFile: vi.fn(() => []),
		reload: vi.fn(),
	}),
}));

const defaultProps = {
	rootPath: "/repo",
	worktreeName: "test-worktree",
};

describe("RightSidebarBottom", () => {
	it("should render terminal panel", () => {
		render(<RightSidebarBottom {...defaultProps} />);

		expect(screen.getByTestId("panel-terminal")).toBeInTheDocument();
		expect(screen.getByTestId("resizable-group")).toBeInTheDocument();
	});

	it("should render collapse button with correct aria-label when expanded", () => {
		render(
			<RightSidebarBottom
				{...defaultProps}
				collapsed={false}
				onToggleCollapse={vi.fn()}
			/>,
		);

		expect(
			screen.getByRole("button", { name: "Collapse panel" }),
		).toBeInTheDocument();
	});

	it("should render expand button with correct aria-label when collapsed", () => {
		render(
			<RightSidebarBottom
				{...defaultProps}
				collapsed={true}
				onToggleCollapse={vi.fn()}
			/>,
		);

		expect(
			screen.getByRole("button", { name: "Expand panel" }),
		).toBeInTheDocument();
	});

	it("should call onToggleCollapse when collapse button is clicked", async () => {
		const user = userEvent.setup();
		const onToggleCollapse = vi.fn();

		render(
			<RightSidebarBottom
				{...defaultProps}
				collapsed={false}
				onToggleCollapse={onToggleCollapse}
			/>,
		);

		await user.click(screen.getByRole("button", { name: "Collapse panel" }));
		expect(onToggleCollapse).toHaveBeenCalledOnce();
	});

	it("should not render collapse button when onToggleCollapse is not provided", () => {
		render(<RightSidebarBottom {...defaultProps} />);

		expect(
			screen.queryByRole("button", { name: "Collapse panel" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Expand panel" }),
		).not.toBeInTheDocument();
	});
});
