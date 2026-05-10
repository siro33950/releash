import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { BackendInfo } from "@/types/session";
import { BackendSelector } from "./BackendSelector";

const makeBackend = (
	overrides: Partial<BackendInfo> & { id: string; name: string },
): BackendInfo => ({
	available: true,
	...overrides,
});

describe("BackendSelector", () => {
	it("returns null when backends is empty", () => {
		const { container } = render(
			<BackendSelector
				backends={[]}
				selectedBackendId={null}
				onBackendChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(container.innerHTML).toBe("");
	});

	it("returns null when backends has only one entry", () => {
		const { container } = render(
			<BackendSelector
				backends={[makeBackend({ id: "b1", name: "Claude" })]}
				selectedBackendId="b1"
				onBackendChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(container.innerHTML).toBe("");
	});

	it("renders selector when multiple backends are provided", () => {
		render(
			<BackendSelector
				backends={[
					makeBackend({ id: "b1", name: "Claude" }),
					makeBackend({ id: "b2", name: "GPT" }),
				]}
				selectedBackendId="b1"
				onBackendChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("backend-selector-trigger")).toBeDefined();
	});

	it("displays the selected backend name on the trigger button", () => {
		render(
			<BackendSelector
				backends={[
					makeBackend({ id: "b1", name: "Claude" }),
					makeBackend({ id: "b2", name: "GPT" }),
				]}
				selectedBackendId="b2"
				onBackendChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("backend-selector-trigger")).toHaveTextContent(
			"GPT",
		);
	});

	it("falls back to the first backend name when selectedBackendId is null", () => {
		render(
			<BackendSelector
				backends={[
					makeBackend({ id: "b1", name: "Claude" }),
					makeBackend({ id: "b2", name: "GPT" }),
				]}
				selectedBackendId={null}
				onBackendChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("backend-selector-trigger")).toHaveTextContent(
			"Claude",
		);
	});

	it("disables the trigger button when disabled is true", () => {
		render(
			<BackendSelector
				backends={[
					makeBackend({ id: "b1", name: "Claude" }),
					makeBackend({ id: "b2", name: "GPT" }),
				]}
				selectedBackendId="b1"
				onBackendChange={vi.fn()}
				disabled={true}
			/>,
		);
		expect(screen.getByTestId("backend-selector-trigger")).toBeDisabled();
	});
});
