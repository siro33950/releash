import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	getTerminalStreamEndpoint,
	resetTerminalStreamEndpointCache,
} from "./terminalStreamEndpoint";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

describe("getTerminalStreamEndpoint", () => {
	beforeEach(() => {
		invokeMock.mockReset();
		resetTerminalStreamEndpointCache();
	});

	it("backendのendpointを返しresultをcacheする", async () => {
		invokeMock.mockResolvedValue({
			url: "ws://127.0.0.1:9999/v1/terminal",
			authSubprotocol: "releash-bearer.test-token",
		});

		const first = await getTerminalStreamEndpoint();
		const second = await getTerminalStreamEndpoint();

		expect(first).toEqual({
			url: "ws://127.0.0.1:9999/v1/terminal",
			authSubprotocol: "releash-bearer.test-token",
		});
		expect(second).toBe(first);
		expect(invokeMock).toHaveBeenCalledTimes(1);
		expect(invokeMock).toHaveBeenCalledWith("get_terminal_stream_endpoint");
	});

	it("endpoint未提供（null・空url）はnullへ正規化する", async () => {
		invokeMock.mockResolvedValueOnce(null);
		expect(await getTerminalStreamEndpoint()).toBeNull();

		resetTerminalStreamEndpointCache();
		invokeMock.mockResolvedValueOnce({
			url: "",
			authSubprotocol: "releash-bearer.test-token",
		});
		expect(await getTerminalStreamEndpoint()).toBeNull();
	});

	it("invoke失敗時はnullへfallbackしcacheを破棄して次回再試行する", async () => {
		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
		invokeMock.mockRejectedValueOnce(new Error("unavailable"));

		expect(await getTerminalStreamEndpoint()).toBeNull();
		expect(warnSpy).toHaveBeenCalledTimes(1);

		invokeMock.mockResolvedValueOnce({
			url: "ws://127.0.0.1:9999/v1/terminal",
			authSubprotocol: "releash-bearer.test-token",
		});
		const retried = await getTerminalStreamEndpoint();

		expect(retried).toEqual({
			url: "ws://127.0.0.1:9999/v1/terminal",
			authSubprotocol: "releash-bearer.test-token",
		});
		expect(invokeMock).toHaveBeenCalledTimes(2);
		warnSpy.mockRestore();
	});
});
