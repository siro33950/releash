import { describe, expect, it } from "vitest";
import { compareCanonicalDecimal } from "./canonicalDecimal";

describe("compareCanonicalDecimal", () => {
	it("B075 compares the lossless 0, 1, and i64 maximum boundaries", () => {
		expect(compareCanonicalDecimal("0", "0")).toBe(0);
		expect(compareCanonicalDecimal("0", "1")).toBe(-1);
		expect(compareCanonicalDecimal("1", "0")).toBe(1);
		expect(
			compareCanonicalDecimal("9007199254740992", "9007199254740993"),
		).toBe(-1);
		expect(
			compareCanonicalDecimal("9223372036854775807", "9223372036854775807"),
		).toBe(0);
		expect(compareCanonicalDecimal("9", "10")).toBe(-1);
	});

	it.each(["", "01", "+1", "-1", "1e0", "１", " 1", "1 "])(
		"B075 rejects non-canonical input %j",
		(value) => {
			expect(() => compareCanonicalDecimal(value, "1")).toThrow(
				"expected canonical nonnegative decimal strings",
			);
			expect(() => compareCanonicalDecimal("1", value)).toThrow(
				"expected canonical nonnegative decimal strings",
			);
		},
	);
});
