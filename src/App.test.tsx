import { invoke } from "@tauri-apps/api/core";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

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

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
	mockInvoke.mockRejectedValue(new Error("not in a git repo"));
});

describe("App", () => {
	it("renders manager screen by default", async () => {
		render(<App />);
		await waitFor(() => {
			expect(screen.getByText("Open Folder")).toBeInTheDocument();
		});
	});
});
