import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invokeClient, subscribeState } from "@/lib/client";
import { useNotionLabelOptions } from "./useNotionLabelOptions";

beforeEach(() => {
	vi.clearAllMocks();
	vi.mocked(subscribeState).mockImplementation(() => vi.fn());
});
it("repoを購読しpeopleの識別子を含む選択肢と更新を表示する", () => {
	const { result, unmount } = renderHook(() => useNotionLabelOptions("/repo"));
	expect(subscribeState).toHaveBeenCalledWith(
		{ kind: "notion-label-options", args: ["/repo"] },
		expect.any(Function),
		expect.any(Function),
	);
	expect(result.current.loading).toBe(true);
	const [, receive] = vi.mocked(subscribeState).mock.calls[0];
	const options = [
		{
			property_name: "Assignee",
			property_type: "people",
			options: ["Alice"],
			option_ids: ["uuid"],
		},
	];
	act(() => receive({ options }));
	expect(result.current.labelOptions).toEqual(options);
	expect(result.current.loading).toBe(false);
	act(() => receive({ options: [] }));
	expect(result.current.labelOptions).toEqual([]);
	expect(invokeClient).not.toHaveBeenCalled();
	const release = vi.mocked(subscribeState).mock.results[0].value;
	unmount();
	expect(release).toHaveBeenCalledOnce();
});
it("前の選択肢と取得失敗を表示し初回失敗と回復も扱う", () => {
	const { result } = renderHook(() => useNotionLabelOptions("/repo"));
	const [, receive, fail] = vi.mocked(subscribeState).mock.calls[0];
	act(() =>
		receive({
			readError: { code: 9, message: "not configured", configMissing: true },
		}),
	);
	expect(result.current.labelOptions).toEqual([]);
	expect(result.current.loading).toBe(false);
	expect(result.current.readError?.code).toBe(9);
	expect(result.current.readError?.configMissing).toBe(true);
	const options = [
		{
			property_name: "Status",
			property_type: "status",
			options: ["Todo"],
			option_ids: [],
		},
	];
	act(() => receive({ options, readError: { code: 13, message: "offline" } }));
	expect(result.current.labelOptions).toEqual(options);
	expect(result.current.error).toBe("offline");
	act(() => fail(new Error("stream failed")));
	expect(result.current.labelOptions).toEqual(options);
	act(() => receive({ options }));
	expect(result.current.error).toBeNull();
});
