import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activateTerminalLink } from "./terminalLinkActivation";

const mocks = vi.hoisted(() => ({
	openUrl: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

describe("activateTerminalLink", () => {
	beforeEach(() => {
		mocks.openUrl.mockReset().mockResolvedValue(undefined);
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("受け取ったURLをopenUrlへそのまま渡す", () => {
		const url = "https://example.com/path?query=value#fragment";

		activateTerminalLink(url);

		expect(mocks.openUrl).toHaveBeenCalledOnce();
		expect(mocks.openUrl).toHaveBeenCalledWith(url);
	});

	it("window.openを呼ばない", () => {
		const windowOpen = vi.spyOn(window, "open");

		activateTerminalLink("https://example.com");

		expect(windowOpen).not.toHaveBeenCalled();
	});

	it("openUrlの失敗を記録し呼び出し元へ例外を伝播させない", async () => {
		const error = new Error("open failed");
		const consoleError = vi
			.spyOn(console, "error")
			.mockImplementation(() => {});
		mocks.openUrl.mockRejectedValueOnce(error);

		expect(() => activateTerminalLink("https://example.com")).not.toThrow();
		await Promise.resolve();

		expect(consoleError).toHaveBeenCalledWith(
			"Failed to open terminal link:",
			error,
		);
	});
});
