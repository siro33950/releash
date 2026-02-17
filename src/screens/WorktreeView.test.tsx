import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "@/types/settings";
import { WorktreeView } from "./WorktreeView";

vi.mock("@/hooks/useCurrentBranch", () => ({
	useCurrentBranch: () => ({ branch: "main", refresh: vi.fn() }),
}));

vi.mock("@/hooks/useFileWatcher", () => ({
	useFileWatcher: vi.fn().mockReturnValue({
		watcherId: 1,
		isWatching: true,
		error: null,
	}),
}));

vi.mock("flexlayout-react", () => {
	const Model = {
		fromJson: () => ({
			getNodeById: () => null,
			doAction: vi.fn(),
		}),
	};
	const Actions = {
		DELETE_TAB: "FlexLayout_DeleteTab",
		addNode: vi.fn(),
		deleteTab: vi.fn(),
		selectTab: vi.fn(),
		updateNodeAttributes: vi.fn(),
	};
	const DockLocation = { CENTER: "center" };
	const Layout = ({
		factory: _factory,
	}: {
		factory: (node: unknown) => unknown;
	}) => {
		return <div data-testid="flexlayout" />;
	};
	return { Model, Actions, DockLocation, Layout };
});

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

describe("WorktreeView", () => {
	it("renders without crashing", () => {
		render(
			<WorktreeView
				rootPath="/test/path"
				settings={DEFAULT_SETTINGS}
				onSettingsSave={vi.fn()}
				onSwitchToKanban={vi.fn()}
				isActive
			/>,
		);
		expect(screen.getByTestId("flexlayout")).toBeInTheDocument();
	});

	it("renders toggle buttons for sidebar, review, terminal", () => {
		render(
			<WorktreeView
				rootPath="/test/path"
				settings={DEFAULT_SETTINGS}
				onSettingsSave={vi.fn()}
				onSwitchToKanban={vi.fn()}
				isActive
			/>,
		);
		expect(screen.getByLabelText("Toggle Sidebar")).toBeInTheDocument();
		expect(screen.getByLabelText("Toggle Review")).toBeInTheDocument();
		expect(screen.getByLabelText("Toggle Terminal")).toBeInTheDocument();
	});
});
