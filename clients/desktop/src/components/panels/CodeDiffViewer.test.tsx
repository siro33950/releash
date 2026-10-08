import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient as invoke } from "@/lib/client";
import type { Hunk } from "@/lib/computeHunks";
import { CodeDiffViewer } from "./CodeDiffViewer";

const mocks = vi.hoisted(() => ({
	shikiDiffViewer: vi.fn((_props: unknown) => null),
}));

vi.mock("@/lib/client", () => ({
	invokeClient: vi.fn(),
}));

vi.mock("./ShikiDiffViewer", () => ({
	ShikiDiffViewer: mocks.shikiDiffViewer,
}));

describe("CodeDiffViewer", () => {
	beforeEach(() => mocks.shikiDiffViewer.mockClear());
	it("言語判定に失敗したときに失敗を表示する", async () => {
		vi.mocked(invoke).mockRejectedValueOnce(new Error("language unavailable"));
		render(
			<CodeDiffViewer
				originalContent="before"
				modifiedContent="after"
				diffMode="inline"
				filePath="src/app.ts"
				hunks={[]}
			/>,
		);
		expect(await screen.findByRole("alert")).toHaveTextContent(
			"language unavailable",
		);
	});

	it("passes provided hunks through to ShikiDiffViewer without computing them in frontend", () => {
		const hunks: Hunk[] = [
			{
				index: 3,
				hunkId: "h:provided:0",
				oldStart: 1,
				oldLines: 1,
				newStart: 1,
				newLines: 2,
				lines: ["@@ -1 +1,2 @@", "-old", "+new"],
			},
		];

		render(
			<CodeDiffViewer
				originalContent="old"
				modifiedContent="new"
				diffMode="inline"
				language="typescript"
				filePath="src/app.ts"
				hunks={hunks}
			/>,
		);

		const props = mocks.shikiDiffViewer.mock.calls[0]?.[0] as
			| { hunks: Hunk[] }
			| undefined;
		expect(props).toBeDefined();
		expect(props?.hunks).toBe(hunks);
		expect(invoke).not.toHaveBeenCalledWith(
			"compute_diff_hunks",
			expect.anything(),
		);
	});
});
