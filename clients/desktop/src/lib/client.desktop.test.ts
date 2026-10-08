import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.unmock("@/lib/client");
let invoke: typeof import("@tauri-apps/api/core").invoke;
let connectFixture: typeof import("@/test/connect").connectFixture;
let invokeClient: typeof import("./client").invokeClient;
beforeEach(async () => {
	vi.resetModules();
	({ invoke } = await import("@tauri-apps/api/core"));
	({ connectFixture } = await import("@/test/connect"));
	({ invokeClient } = await import("./client"));
});
afterEach(async () => {
	window.dispatchEvent(new Event("pagehide"));
	await new Promise((resolve) => setTimeout(resolve, 0));
	vi.unstubAllGlobals();
});

it("desktopの業務要求をHTTPへ送りIPCには接続情報と同一性検証だけを渡す", async () => {
	vi.mocked(invoke).mockClear();
	const fixture = connectFixture({
		addRepoPath: () => ({ value: true }),
	});
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	for (const request of fixture.requests)
		expect(request.headers.get("authorization")).toBe("Bearer client-token");
	expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).toEqual([
		"get_client_endpoint",
	]);
	expect(invoke).toHaveBeenCalledWith("get_client_endpoint");
});

it("破棄済み画面の遅い接続情報が次の接続を上書きしない", async () => {
	connectFixture({ addRepoPath: () => ({ value: true }) });
	const original = vi.mocked(invoke).getMockImplementation();
	if (!original) throw new Error("Missing endpoint fixture");
	let release!: (value: unknown) => void;
	let endpoints = 0;
	vi.mocked(invoke).mockImplementation((command, args) => {
		if (command === "get_client_endpoint" && ++endpoints === 1)
			return new Promise((resolve) => {
				release = resolve;
			});
		return original(command, args);
	});
	const old = invokeClient("add_repo_path", { path: "/repo" }).catch(
		(error) => error,
	);
	window.dispatchEvent(
		new PageTransitionEvent("pagehide", { persisted: true }),
	);
	window.dispatchEvent(
		new PageTransitionEvent("pageshow", { persisted: true }),
	);
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	release({ url: "http://127.0.0.1:9829", token: "old" });
	expect(await old).toBeInstanceOf(Error);
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	expect(endpoints).toBe(2);
});

it("Rustの発見検証が失敗した接続では業務RPCを呼ばない", async () => {
	const read = vi.fn(() => ({ value: true }));
	connectFixture({ addRepoPath: read });
	vi.mocked(invoke).mockRejectedValue(new Error("Daemon identity changed"));
	await expect(
		invokeClient("add_repo_path", { path: "/repo" }),
	).rejects.toThrow("Daemon identity changed");
	expect(read).not.toHaveBeenCalled();
});

it.each(["settings denied", "deadline has elapsed"])(
	"初回desktop設定の失敗(%s)ではREADYにならず業務要求を送らない",
	async (message) => {
		const read = vi.fn(() => ({ value: true }));
		connectFixture({ addRepoPath: read });
		const { getClient, getConnectionState } = await import("./client");
		vi.mocked(invoke).mockRejectedValue(new Error(message));
		await expect(getClient()).rejects.toThrow(message);
		expect(getConnectionState()).toBe("TRANSIENT_FAILURE");
		await expect(
			invokeClient("add_repo_path", { path: "/repo" }),
		).rejects.toThrow("Daemon connection is TRANSIENT_FAILURE");
		expect(read).not.toHaveBeenCalled();
	},
);
