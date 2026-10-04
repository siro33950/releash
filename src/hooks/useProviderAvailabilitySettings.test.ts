import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useProviderAvailabilitySettings } from "./useProviderAvailabilitySettings";

const states = stateSubscriptions();
const savedExecutable = () => Promise.resolve(null as never);
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

describe("useProviderAvailabilitySettings", () => {
	beforeEach(async () => {
		states.clear();
		const { invokeClient } = await import("@/lib/client");
		vi.mocked(invokeClient).mockReset();
	});

	it.each([
		[
			{ code: "PROVIDER_AVAILABILITY_CORRUPT", message: "backend message" },
			"backend message",
		],
		["plain message", "plain message"],
	])(
		"Provider executable設定の取得失敗からmessageだけを保持する",
		async (rejection, expected) => {
			const { result } = renderHook(() =>
				useProviderAvailabilitySettings(true),
			);
			expect(result.current.loading).toBe(true);
			act(() => states.fail("provider-availability", rejection));

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
				expect(result.current.error).toBe(expected);
			});
			expect(states.subscribeState).toHaveBeenCalledWith(
				"provider-availability",
				expect.any(Function),
				expect.any(Function),
			);
		},
	);

	it("閉じている間は購読せず開くと届いた一覧を表示する", async () => {
		const provider = {
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: null,
			configurationRevision: 0,
			effectiveExecutable: "claude",
			available: true,
			resolvedExecutable: "/usr/bin/claude",
			unavailableReason: null,
		};
		states.publish("provider-availability", { providers: [provider] });
		const { result, rerender } = renderHook(
			({ open }) => useProviderAvailabilitySettings(open),
			{ initialProps: { open: false } },
		);
		expect(states.subscribeState).not.toHaveBeenCalled();
		expect(result.current.loading).toBe(false);
		rerender({ open: true });
		await waitFor(() => expect(result.current.providers).toEqual([provider]));
		expect(result.current.drafts).toEqual({ claude: "" });
	});

	it("resetの応答では一覧を変えず購読を受けてから更新する", async () => {
		const { invokeClient } = await import("@/lib/client");
		const claude = {
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: "/custom/claude",
			configurationRevision: 0,
			effectiveExecutable: "/custom/claude",
			available: true,
			resolvedExecutable: "/custom/claude",
			unavailableReason: null,
		};
		const codex = {
			provider: "codex",
			displayName: "Codex",
			defaultExecutable: "codex",
			configuredExecutable: null,
			configurationRevision: 0,
			effectiveExecutable: "codex",
			available: true,
			resolvedExecutable: "/bin/codex",
			unavailableReason: null,
		};
		states.publish("provider-availability", { providers: [claude, codex] });
		vi.mocked(invokeClient).mockResolvedValue({ providers: [] } as never);
		const { result } = renderHook(() => useProviderAvailabilitySettings(true));
		await waitFor(() => expect(result.current.providers).toHaveLength(2));
		act(() => result.current.setExecutable("codex", "/draft/codex"));
		await act(async () => result.current.reset("claude"));
		expect(result.current.providers).toEqual([claude, codex]);
		act(() =>
			states.publish("provider-availability", {
				providers: [
					{
						...claude,
						configuredExecutable: null,
						configurationRevision: 0,
						effectiveExecutable: "claude",
					},
					codex,
				],
			}),
		);
		await waitFor(() =>
			expect(result.current.providers[0].configuredExecutable).toBeNull(),
		);
		expect(result.current.drafts.codex).toBe("/draft/codex");
	});

	it("保存後は購読の正規化値を入力へ反映し他providerの未保存入力を残す", async () => {
		const { invokeClient } = await import("@/lib/client");
		const provider = (id: string, configuredExecutable: string | null) => ({
			provider: id,
			displayName: id,
			defaultExecutable: id,
			configuredExecutable,
			configurationRevision: 0,
			effectiveExecutable: configuredExecutable ?? id,
			available: true,
			resolvedExecutable: `/bin/${id}`,
			unavailableReason: null,
		});
		states.publish("provider-availability", {
			providers: [provider("claude", null), provider("codex", null)],
		});
		vi.mocked(invokeClient).mockImplementation(savedExecutable);
		const { result } = renderHook(() => useProviderAvailabilitySettings(true));
		await waitFor(() => expect(result.current.providers).toHaveLength(2));
		act(() => result.current.setExecutable("claude", " /x "));
		let save: Promise<void>;
		act(() => {
			save = result.current.save();
			result.current.setExecutable("codex", "/draft/codex");
		});
		await act(async () => save);
		expect(result.current.drafts.claude).toBe(" /x ");
		act(() =>
			states.publish("provider-availability", {
				providers: [provider("claude", "/x"), provider("codex", null)],
			}),
		);
		expect(result.current.drafts).toEqual({
			claude: "/x",
			codex: "/draft/codex",
		});
		vi.mocked(invokeClient).mockClear();
		await act(async () => result.current.save());
		expect(invokeClient).toHaveBeenCalledTimes(1);
		expect(invokeClient).toHaveBeenCalledWith("update_provider_executable", {
			provider: "codex",
			executable: "/draft/codex",
		});
	});

	it("reset応答が購読より先でも古い設定と入力を保ち空文字の保存を送らない", async () => {
		const { invokeClient } = await import("@/lib/client");
		const provider = {
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: "/custom/claude",
			configurationRevision: 0,
			effectiveExecutable: "/custom/claude",
			available: true,
			resolvedExecutable: "/custom/claude",
			unavailableReason: null,
		};
		states.publish("provider-availability", { providers: [provider] });
		vi.mocked(invokeClient).mockResolvedValue(null as never);
		const { result } = renderHook(() => useProviderAvailabilitySettings(true));
		await waitFor(() => expect(result.current.providers).toHaveLength(1));
		await act(async () => result.current.reset("claude"));
		expect(result.current.drafts.claude).toBe("/custom/claude");
		expect(result.current.isDirty).toBe(false);
		vi.mocked(invokeClient).mockClear();
		await act(async () => result.current.save());
		expect(invokeClient).not.toHaveBeenCalled();
		act(() =>
			states.publish("provider-availability", {
				providers: [{ ...provider, configuredExecutable: null }],
			}),
		);
		expect(result.current.drafts.claude).toBe("");
		expect(result.current.isDirty).toBe(false);
	});

	it("保存前と同じ値へ正規化された購読が応答より先でも保存済み入力に揃える", async () => {
		const { invokeClient } = await import("@/lib/client");
		const provider = {
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: "/x",
			configurationRevision: 0,
			effectiveExecutable: "/x",
			available: true,
			resolvedExecutable: "/x",
			unavailableReason: null,
		};
		states.publish("provider-availability", { providers: [provider] });
		let resolve!: (value: string) => void;
		vi.mocked(invokeClient).mockReturnValue(
			new Promise<string>((done) => {
				resolve = done;
			}) as never,
		);
		const { result } = renderHook(() => useProviderAvailabilitySettings(true));
		await waitFor(() => expect(result.current.providers).toHaveLength(1));
		act(() => result.current.setExecutable("claude", " /x "));
		let save!: Promise<void>;
		act(() => {
			save = result.current.save();
		});
		act(() =>
			states.publish("provider-availability", {
				providers: [{ ...provider, configurationRevision: 1 }],
			}),
		);
		expect(result.current.drafts.claude).toBe(" /x ");
		await act(async () => {
			resolve("/x");
			await save;
		});
		expect(result.current.drafts.claude).toBe("/x");
		expect(result.current.isDirty).toBe(false);
	});

	it("同じ設定への正規化保存も購読revisionで同期し再保存しない", async () => {
		const { invokeClient } = await import("@/lib/client");
		const provider = {
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: "/x",
			configurationRevision: 0,
			effectiveExecutable: "/x",
			available: true,
			resolvedExecutable: "/x",
			unavailableReason: null,
		};
		states.publish("provider-availability", { providers: [provider] });
		vi.mocked(invokeClient).mockImplementation(savedExecutable);
		const { result } = renderHook(() => useProviderAvailabilitySettings(true));
		await waitFor(() => expect(result.current.providers).toHaveLength(1));
		act(() => result.current.setExecutable("claude", " /x "));
		expect(result.current.isDirty).toBe(true);
		await act(async () => result.current.save());
		expect(result.current.drafts.claude).toBe(" /x ");
		expect(result.current.isDirty).toBe(true);
		act(() =>
			states.publish("provider-availability", {
				providers: [{ ...provider, configurationRevision: 1 }],
			}),
		);
		expect(result.current.drafts.claude).toBe("/x");
		expect(result.current.isDirty).toBe(false);
		vi.mocked(invokeClient).mockClear();
		await act(async () => result.current.save());
		expect(invokeClient).not.toHaveBeenCalled();
	});

	it("複数providerの保存中に先の購読が届いても次の未保存入力を保つ", async () => {
		const { invokeClient } = await import("@/lib/client");
		const provider = (id: string, configuredExecutable: string | null) => ({
			provider: id,
			displayName: id,
			defaultExecutable: id,
			configuredExecutable,
			configurationRevision: 0,
			effectiveExecutable: configuredExecutable ?? id,
			available: true,
			resolvedExecutable: configuredExecutable,
			unavailableReason: null,
		});
		states.publish("provider-availability", {
			providers: [provider("claude", null), provider("codex", null)],
		});
		let resolveCodex!: (value: string) => void;
		vi.mocked(invokeClient).mockImplementation((_command, args) =>
			args && "provider" in args && args.provider === "codex"
				? (new Promise<string>((done) => {
						resolveCodex = done;
					}) as never)
				: savedExecutable(),
		);
		const { result } = renderHook(() => useProviderAvailabilitySettings(true));
		await waitFor(() => expect(result.current.providers).toHaveLength(2));
		act(() => result.current.setExecutable("claude", " /x "));
		act(() => result.current.setExecutable("codex", "/draft/codex"));
		let save!: Promise<void>;
		act(() => {
			save = result.current.save();
		});
		await waitFor(() => expect(invokeClient).toHaveBeenCalledTimes(2));
		act(() =>
			states.publish("provider-availability", {
				providers: [provider("claude", "/x"), provider("codex", null)],
			}),
		);
		expect(result.current.drafts).toEqual({
			claude: "/x",
			codex: "/draft/codex",
		});
		await act(async () => {
			resolveCodex("/draft/codex");
			await save;
		});
		act(() =>
			states.publish("provider-availability", {
				providers: [
					provider("claude", "/x"),
					provider("codex", "/draft/codex"),
				],
			}),
		);
		expect(result.current.isDirty).toBe(false);
	});
	it.each(["save", "reset"] as const)(
		"%s失敗後はエラーと両providerの入力を保ち次の購読でも消さない",
		async (operation) => {
			const { invokeClient } = await import("@/lib/client");
			const provider = (id: string, configuredExecutable: string | null) => ({
				provider: id,
				displayName: id,
				defaultExecutable: id,
				configuredExecutable,
				configurationRevision: 0,
				effectiveExecutable: configuredExecutable ?? id,
				available: true,
				resolvedExecutable: null,
				unavailableReason: null,
			});
			const providers = [provider("claude", "/old"), provider("codex", null)];
			states.publish("provider-availability", { providers });
			vi.mocked(invokeClient).mockRejectedValue(new Error("operation failed"));
			const { result } = renderHook(() =>
				useProviderAvailabilitySettings(true),
			);
			await waitFor(() => expect(result.current.providers).toHaveLength(2));
			act(() => result.current.setExecutable("claude", "/draft/claude"));
			act(() => result.current.setExecutable("codex", "/draft/codex"));
			await act(async () => {
				if (operation === "save") {
					await expect(result.current.save()).rejects.toThrow(
						"operation failed",
					);
				} else {
					await result.current.reset("claude");
				}
			});
			expect(invokeClient).toHaveBeenCalledWith(
				operation === "save"
					? "update_provider_executable"
					: "reset_provider_executable",
				expect.objectContaining({ provider: "claude" }),
			);
			expect(result.current.error).toBe("operation failed");
			expect(result.current.drafts).toEqual({
				claude: "/draft/claude",
				codex: "/draft/codex",
			});
			act(() =>
				states.publish("provider-availability", {
					providers: [
						provider("claude", "/server/claude"),
						provider("codex", "/server/codex"),
					],
				}),
			);
			expect(result.current.drafts).toEqual({
				claude: "/draft/claude",
				codex: "/draft/codex",
			});
		},
	);
	it.each([false, true])(
		"同じ設定への保存で追加編集した入力は保持する: subscriptionFirst=%s",
		async (subscriptionFirst) => {
			const { invokeClient } = await import("@/lib/client");
			const provider = {
				provider: "claude",
				displayName: "Claude",
				defaultExecutable: "claude",
				configuredExecutable: "/x",
				configurationRevision: 0,
				effectiveExecutable: "/x",
				available: true,
				resolvedExecutable: "/x",
				unavailableReason: null,
			};
			states.publish("provider-availability", { providers: [provider] });
			let complete!: () => void;
			vi.mocked(invokeClient).mockReturnValue(
				new Promise((resolve) => {
					complete = () => resolve(null as never);
				}) as never,
			);
			const { result } = renderHook(() =>
				useProviderAvailabilitySettings(true),
			);
			act(() => result.current.setExecutable("claude", " /x "));
			let save!: Promise<void>;
			act(() => {
				save = result.current.save();
			});
			act(() => result.current.setExecutable("claude", "/additional"));
			const publish = () =>
				states.publish("provider-availability", {
					providers: [{ ...provider, configurationRevision: 1 }],
				});
			if (subscriptionFirst) act(publish);
			await act(async () => {
				complete();
				await save;
			});
			if (!subscriptionFirst) act(publish);
			expect(result.current.drafts.claude).toBe("/additional");
			expect(result.current.isDirty).toBe(true);
		},
	);
});
