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
	const Layout = ({
		factory,
	}: {
		factory: (node: { getComponent: () => string }) => React.ReactNode;
	}) => {
		const components = ["sidebar", "kanban", "terminal"];
		return (
			<div data-testid="flexlayout">
				{components.map((c) => (
					<div key={c}>{factory({ getComponent: () => c })}</div>
				))}
			</div>
		);
	};
	return { Model, Layout };
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
