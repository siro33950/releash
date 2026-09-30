import { invoke } from "@tauri-apps/api/core";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient, subscribeState } from "@/lib/client";
import App from "./App";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const failed = {
	type: "failed",
	kind: "store_validation_failed",
	safeDescription: "The local data store could not be verified safely.",
	correlationId: "startup-correlation-1",
	retryOnNextLaunch: false,
	actions: ["quit"],
} as const;

describe("B-071 safe startup surface", () => {
	beforeEach(() => {
		vi.mocked(invoke).mockReset();
		vi.mocked(invoke).mockImplementation(async (command: string, args) => {
			if (command === "subscribe_daemon_status") {
				(
					args as { channel: { onmessage?: (value: unknown) => void } }
				).channel.onmessage?.({ phase: "ready", connectionGeneration: 1 });
				return;
			}
			throw new Error(`unexpected shell command: ${command}`);
		});
		vi.mocked(invokeClient).mockReset();
	});

	it("mounts no workbench and exposes only safe failure data and Quit", async () => {
		vi.mocked(subscribeState).mockImplementation((target, receive) => {
			const kind = typeof target === "string" ? target : target.kind;
			if (kind === "startup-outcome") receive(failed as never);
			else throw new Error(`unexpected subscription: ${kind}`);
			return () => {};
		});
		vi.mocked(invokeClient).mockImplementation(async (command) => {
			if (command === "quit_after_startup_failure")
				return {
					type: "accepted",
					correlationId: "startup-correlation-1",
				} as never;
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
		expect(invokeClient).not.toHaveBeenCalled();

		await userEvent.click(screen.getByRole("button", { name: "Quit" }));
		await waitFor(() =>
			expect(invokeClient).toHaveBeenLastCalledWith(
				"quit_after_startup_failure",
			),
		);
		expect(invokeClient).toHaveBeenCalledTimes(1);
		expect(
			vi
				.mocked(invoke)
				.mock.calls.every(([command]) => command === "subscribe_daemon_status"),
		).toBe(true);
	});

	it("Quitの呼び出し失敗を画面に表示する", async () => {
		vi.mocked(subscribeState).mockImplementation((target, receive) => {
			if (
				(typeof target === "string" ? target : target.kind) ===
				"startup-outcome"
			)
				receive(failed as never);
			return () => {};
		});
		vi.mocked(invokeClient).mockRejectedValue(new Error("Quit failed"));
		render(<App />);
		await userEvent.click(await screen.findByRole("button", { name: "Quit" }));
		expect(await screen.findByRole("alert")).toHaveTextContent("Quit failed");
	});

	it("does not synthesize a failure kind, description, correlation, or Quit when the Rust outcome is unavailable", async () => {
		vi.mocked(subscribeState).mockImplementation(
			(target, _receive, onError) => {
				const kind = typeof target === "string" ? target : target.kind;
				if (kind === "startup-outcome")
					onError?.(new Error("startup authority missing"));
				else throw new Error(`unexpected subscription: ${kind}`);
				return () => {};
			},
		);

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
		expect(invokeClient).not.toHaveBeenCalled();
	});
});
