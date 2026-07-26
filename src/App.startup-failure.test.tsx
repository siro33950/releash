import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

describe("B-071 safe startup surface", () => {
	beforeEach(() => {
		invoke.mockReset();
	});

	it("mounts no workbench and exposes only safe failure data and Quit", async () => {
		invoke.mockImplementation((command: string) => {
			if (command === "get_application_startup_outcome") {
				return Promise.resolve({
					type: "failed",
					kind: "store_validation_failed",
					safeDescription: "The local data store could not be verified safely.",
					correlationId: "startup-correlation-1",
					retryOnNextLaunch: false,
					actions: ["quit"],
				});
			}
			if (command === "quit_after_startup_failure") {
				return Promise.resolve({
					type: "accepted",
					correlationId: "startup-correlation-1",
				});
			}
			throw new Error(`unexpected normal command: ${command}`);
		});

		render(<App />);

		expect(
			await screen.findByRole("heading", {
				name: "The local data store could not be verified safely.",
			}),
		).toBeInTheDocument();
		expect(
			screen.getByText("store_validation_failed", { selector: "code" }),
		).toBeInTheDocument();
		expect(
			screen.getByText("Correlation: startup-correlation-1"),
		).toBeInTheDocument();
		expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
		expect(invoke).toHaveBeenCalledTimes(1);

		await userEvent.click(screen.getByRole("button", { name: "Quit" }));
		await waitFor(() =>
			expect(invoke).toHaveBeenLastCalledWith("quit_after_startup_failure"),
		);
		expect(
			invoke.mock.calls.every(
				([command]) =>
					command === "get_application_startup_outcome" ||
					command === "quit_after_startup_failure",
			),
		).toBe(true);
	});

	it("does not synthesize a failure kind, description, correlation, or Quit when the Rust outcome is unavailable", async () => {
		invoke.mockRejectedValueOnce(new Error("startup authority missing"));

		render(<App />);

		expect(
			await screen.findByRole("heading", {
				name: "Startup outcome unavailable",
			}),
		).toBeInTheDocument();
		expect(screen.queryByText(/Correlation:/)).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Quit" }),
		).not.toBeInTheDocument();
		expect(invoke).toHaveBeenCalledTimes(1);
		expect(invoke.mock.calls).toEqual([["get_application_startup_outcome"]]);
	});
});
