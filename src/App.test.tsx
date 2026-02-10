import { invoke } from "@tauri-apps/api/core";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

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
