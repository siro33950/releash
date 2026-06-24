import { invoke } from "@tauri-apps/api/core";
import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Hunk } from "@/lib/computeHunks";
import { CodeDiffViewer } from "./CodeDiffViewer";

const mocks = vi.hoisted(() => ({
	shikiDiffViewer: vi.fn((_props: unknown) => null),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

vi.mock("./ShikiDiffViewer", () => ({
	ShikiDiffViewer: mocks.shikiDiffViewer,
}));

describe("CodeDiffViewer", () => {
	it("passes provided hunks through to ShikiDiffViewer without computing them in frontend", () => {
		const hunks: Hunk[] = [
			{
				index: 3,
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
