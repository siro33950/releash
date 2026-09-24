import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useStateSubscriptionResult } from "./useStateSubscription";

const states = stateSubscriptions();
vi.mock("@/lib/client", () => ({
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));
beforeEach(() => states.clear());

it("同じ対象の購読エラーでは直前値を保持し対象変更では消す", () => {
	const first = { kind: "current-branch" as const, args: ["/repo"] };
	const second = { kind: "current-branch" as const, args: ["/other"] };
	const { result, rerender, unmount } = renderHook(
		({ target }) => useStateSubscriptionResult(target),
		{ initialProps: { target: first } },
	);
	act(() => states.publish(first, "main"));
	act(() => states.fail(first, new Error("offline")));
	expect(result.current).toMatchObject({ value: "main", error: "offline" });
	expect(states.subscribeState).toHaveBeenCalledTimes(1);
	rerender({ target: second });
	expect(result.current.value).toBeUndefined();
	act(() => states.fail(second, new Error("unavailable")));
	expect(result.current).toMatchObject({
		value: undefined,
		error: "unavailable",
	});
	act(() => states.publish(first, "stale"));
	expect(result.current.value).toBeUndefined();
	act(() => states.publish(second, "develop"));
	expect(result.current).toMatchObject({ value: "develop", error: null });
	unmount();
});
