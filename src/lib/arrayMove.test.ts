import { describe, expect, it } from "vitest";
import { arrayMove } from "./arrayMove";

describe("arrayMove", () => {
	it("should move an element forward", () => {
		expect(arrayMove([1, 2, 3, 4], 0, 2)).toEqual([2, 3, 1, 4]);
	});

	it("should move an element backward", () => {
		expect(arrayMove([1, 2, 3, 4], 3, 1)).toEqual([1, 4, 2, 3]);
	});

	it("should return a copy when from === to", () => {
		const arr = [1, 2, 3];
		const result = arrayMove(arr, 1, 1);
		expect(result).toEqual([1, 2, 3]);
		expect(result).not.toBe(arr);
	});

	it("should return a copy for out-of-bounds from", () => {
		expect(arrayMove([1, 2, 3], -1, 1)).toEqual([1, 2, 3]);
		expect(arrayMove([1, 2, 3], 5, 1)).toEqual([1, 2, 3]);
	});

	it("should return a copy for out-of-bounds to", () => {
		expect(arrayMove([1, 2, 3], 0, -1)).toEqual([1, 2, 3]);
		expect(arrayMove([1, 2, 3], 0, 5)).toEqual([1, 2, 3]);
	});

	it("should not mutate the original array", () => {
		const arr = [1, 2, 3, 4];
		arrayMove(arr, 0, 3);
		expect(arr).toEqual([1, 2, 3, 4]);
	});

	it("should work with a single element array", () => {
		expect(arrayMove([1], 0, 0)).toEqual([1]);
	});

	it("should work with an empty array", () => {
		expect(arrayMove([], 0, 0)).toEqual([]);
	});
});
