import { describe, expect, it } from "vitest";
import type { Hunk } from "./computeHunks";
import { generatePatch } from "./generatePatch";

describe("generatePatch", () => {
	const sampleHunks: Hunk[] = [
		{
			index: 0,
			oldStart: 1,
			oldLines: 3,
			newStart: 1,
			newLines: 3,
			lines: [" line1", "-original", "+modified", " line3"],
		},
		{
			index: 1,
			oldStart: 8,
			oldLines: 3,
			newStart: 8,
			newLines: 4,
			lines: [" line8", "-old9", "+new9", "+new10", " line11"],
		},
	];

	it("should generate patch for a single hunk", () => {
		const patch = generatePatch("src/file.ts", sampleHunks, [0]);
		expect(patch).toContain("--- a/src/file.ts");
		expect(patch).toContain("+++ b/src/file.ts");
		expect(patch).toContain("@@ -1,3 +1,3 @@");
		expect(patch).toContain("+modified");
		expect(patch).not.toContain("+new9");
	});

	it("should generate patch for multiple hunks", () => {
		const patch = generatePatch("src/file.ts", sampleHunks, [0, 1]);
		expect(patch).toContain("@@ -1,3 +1,3 @@");
		expect(patch).toContain("@@ -8,3 +8,4 @@");
	});

	it("should return empty string for no selected indices", () => {
		const patch = generatePatch("src/file.ts", sampleHunks, []);
		expect(patch).toBe("");
	});

	it("should ignore invalid indices", () => {
		const patch = generatePatch("src/file.ts", sampleHunks, [99]);
		expect(patch).toBe("");
	});

	it("should end with newline", () => {
		const patch = generatePatch("src/file.ts", sampleHunks, [0]);
		expect(patch.endsWith("\n")).toBe(true);
	});
});
