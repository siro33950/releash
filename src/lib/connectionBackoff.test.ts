import { afterEach, expect, it, vi } from "vitest";
import { createConnectionBackoff } from "./connectionBackoff";

afterEach(() => {
	vi.restoreAllMocks();
});

it("初回は1秒でずれ無く、以後1.6倍ずつ伸びて120秒で頭打ちになる", () => {
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	const backoff = createConnectionBackoff();
	expect(backoff.next()).toBe(1000);
	expect(backoff.next()).toBe(1600);
	expect(backoff.next()).toBe(2560);
	let last = 0;
	for (let i = 0; i < 20; i++) last = backoff.next();
	expect(last).toBe(120000);
});

it("2回目以降は±20%の範囲でずらす", () => {
	const random = vi.spyOn(Math, "random");
	const backoff = createConnectionBackoff();
	backoff.next();
	random.mockReturnValue(0);
	expect(backoff.next()).toBe(1280);
	random.mockReturnValue(1);
	expect(backoff.next()).toBe(3072);
});

it("resetで初回の待ちに戻る", () => {
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	const backoff = createConnectionBackoff();
	backoff.next();
	backoff.next();
	backoff.reset();
	expect(backoff.next()).toBe(1000);
});
