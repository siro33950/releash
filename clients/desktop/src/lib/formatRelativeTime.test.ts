import { afterEach, describe, expect, it, vi } from "vitest";
import { formatRelativeTime } from "./formatRelativeTime";

describe("formatRelativeTime", () => {
	afterEach(() => {
		vi.useRealTimers();
	});

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

	it("switches display-only label from now to minutes at 60 seconds", () => {
		const now = new Date("2026-01-01T00:00:00Z");
		vi.useFakeTimers();
		vi.setSystemTime(now);

		expect(formatRelativeTime(now.getTime() - 59000)).toBe("now");
		expect(formatRelativeTime(now.getTime() - 60000)).toBe("1m");
	});
});
