import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StateTarget } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import type { ReviewFileView } from "@/types/review";
import type { DiffSection } from "@/types/settings";
import { useReviewFileView } from "./useReviewFileView";

const states = stateSubscriptions();
vi.mock("@/lib/client", () => ({
	subscribeState: (...args: unknown[]) =>
		(states.subscribeState as (...args: unknown[]) => () => void)(...args),
}));

const target: StateTarget<"review-file-view"> = {
	kind: "review-file-view",
	args: ["/repo", "src/main.ts", "changes", "head"],
};

function textDiff(): ReviewFileView {
	return {
		kind: "textDiff",
		version: 17,
		stale: false,
		fileId: "src/main.ts",
		path: "src/main.ts",
		original: "old",
		modified: "new",
		source: "diff",
		hunks: [],
		changeGroups: [],
		limited: false,
		totalLines: 1,
	};
}

describe("useReviewFileView", () => {
	beforeEach(() => states.clear());

	it("worktree・path・section・baseで購読しテキスト差分を公開する", () => {
		const { result } = renderHook(() =>
			useReviewFileView("/repo", "src/main.ts", "head", "changes"),
		);
		expect(states.subscribeState).toHaveBeenCalledWith(
			target,
			expect.any(Function),
			expect.any(Function),
		);
		expect(result.current.loading).toBe(true);

		act(() => states.publish(target, textDiff()));

		expect(result.current.loading).toBe(false);
		expect(result.current.originalContent).toBe("old");
		expect(result.current.modifiedContent).toBe("new");
		expect(result.current.hunks).toEqual([]);
		expect(result.current.changeGroups).toEqual([]);
		expect(result.current.error).toBeNull();
	});

	it("画像はdaemonが埋め込んだdata URLをそのまま表示する", () => {
		const { result } = renderHook(() =>
			useReviewFileView("/repo", "src/main.ts", "head", "changes"),
		);
		act(() =>
			states.publish(target, {
				kind: "image",
				version: 3,
				stale: false,
				fileId: "src/main.ts",
				path: "src/main.ts",
				originalUrl: "data:image/png;base64,AQID",
				modifiedUrl: null,
				mime: "image/png",
			}),
		);
		expect(result.current.imageDiff).toEqual({
			originalUrl: "data:image/png;base64,AQID",
			modifiedUrl: null,
			loading: false,
		});
		expect(result.current.hunks).toBeNull();
	});

	it("購読の失敗は表示を空にしてerrorへ渡す", () => {
		const { result } = renderHook(() =>
			useReviewFileView("/repo", "src/main.ts", "head", "changes"),
		);
		act(() => states.publish(target, textDiff()));
		act(() => states.fail(target, { message: "read denied" }));
		expect(result.current.view).toBeNull();
		expect(result.current.error).toBe("read denied");
		expect(result.current.loading).toBe(false);
	});

	it("ファイル未選択では購読しない", () => {
		const { result } = renderHook(() =>
			useReviewFileView("/repo", null, "head", "changes"),
		);
		expect(states.subscribeState).not.toHaveBeenCalled();
		expect(result.current.view).toBeNull();
		expect(result.current.loading).toBe(false);
	});

	it("sectionが変わると前の差分を表示せず新しい対象を購読する", () => {
		const { result, rerender } = renderHook(
			({ section }) =>
				useReviewFileView("/repo", "src/main.ts", "head", section),
			{ initialProps: { section: "changes" as DiffSection } },
		);
		act(() => states.publish(target, textDiff()));
		expect(result.current.view?.version).toBe(17);
		rerender({ section: "staged" });
		expect(result.current.view).toBeNull();
		expect(result.current.loading).toBe(true);
		expect(states.subscribeState).toHaveBeenLastCalledWith(
			{
				kind: "review-file-view",
				args: ["/repo", "src/main.ts", "staged", "head"],
			},
			expect.any(Function),
			expect.any(Function),
		);
	});
});
