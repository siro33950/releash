import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
	CodexPermissionControl,
	nextCodexPermissionMode,
} from "./CodexPermissionControl";

describe("CodexPermissionControl", () => {
	it("shows current sandbox label on trigger", () => {
		render(
			<CodexPermissionControl
				mode="acceptEdits"
				onModeChange={vi.fn()}
				disabled={false}
			/>,
		);

		expect(screen.getByTestId("codex-permission-trigger")).toHaveTextContent(
			"Workspace",
		);
	});

	it("maps Read only to plan", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<CodexPermissionControl
				mode="acceptEdits"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("codex-permission-trigger"));
		await user.click(screen.getByText("Read only"));

		expect(onModeChange).toHaveBeenCalledWith("plan");
	});

	it("maps Workspace to acceptEdits", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<CodexPermissionControl
				mode="plan"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("codex-permission-trigger"));
		await user.click(screen.getByText("Workspace"));

		expect(onModeChange).toHaveBeenCalledWith("acceptEdits");
	});

	it("shows selectable approval state in the menu", async () => {
		const user = userEvent.setup();
		render(
			<CodexPermissionControl
				mode="acceptEdits"
				onModeChange={vi.fn()}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("codex-permission-trigger"));

		expect(screen.getByText("Sandbox")).toBeDefined();
		expect(screen.getByText("Approval")).toBeDefined();
		expect(screen.getByText("Ask")).toBeDefined();
		expect(screen.getByText("Never")).toBeDefined();
	});

	it("maps Workspace Approval Ask to default", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<CodexPermissionControl
				mode="acceptEdits"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("codex-permission-trigger"));
		await user.click(screen.getByText("Ask"));

		expect(onModeChange).toHaveBeenCalledWith("default");
	});

	it("maps Workspace Approval Never to acceptEdits", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<CodexPermissionControl
				mode="default"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("codex-permission-trigger"));
		await user.click(screen.getByText("Never"));

		expect(onModeChange).toHaveBeenCalledWith("acceptEdits");
	});

	it("maps Full access to bypassPermissions", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<CodexPermissionControl
				mode="acceptEdits"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("codex-permission-trigger"));
		await user.click(screen.getByText("Full access"));

		expect(onModeChange).toHaveBeenCalledWith("bypassPermissions");
	});

	it("cycles Read only to Workspace to Full access", () => {
		expect(nextCodexPermissionMode("plan")).toBe("default");
		expect(nextCodexPermissionMode("default")).toBe("acceptEdits");
		expect(nextCodexPermissionMode("acceptEdits")).toBe("bypassPermissions");
		expect(nextCodexPermissionMode("bypassPermissions")).toBe("plan");
	});
});
