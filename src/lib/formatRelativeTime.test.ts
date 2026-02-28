import { describe, expect, it } from "vitest";
import { formatRelativeTime } from "./formatRelativeTime";

describe("formatRelativeTime", () => {
	it("should return 'now' for less than 60 seconds", () => {
		expect(formatRelativeTime(Date.now() - 30000)).toBe("now");
	});

	it("should return minutes for less than 1 hour", () => {
		expect(formatRelativeTime(Date.now() - 120000)).toBe("2m");
	});

	it("should return hours for less than 1 day", () => {
		expect(formatRelativeTime(Date.now() - 7200000)).toBe("2h");
	});

	it("should return days for more than 1 day", () => {
		expect(formatRelativeTime(Date.now() - 259200000)).toBe("3d");
	});
});
