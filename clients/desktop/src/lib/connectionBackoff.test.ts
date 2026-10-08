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

it("前回の待ちが終わってから2分ちょうどでは伸び、2分を超えると初回へ戻る", () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const backoff = createConnectionBackoff();
		expect(backoff.next()).toBe(1000);
		vi.setSystemTime(1000);
		backoff.attemptStarted();
		vi.setSystemTime(121000);
		expect(backoff.next()).toBe(1600);
		vi.setSystemTime(122600);
		backoff.attemptStarted();
		vi.setSystemTime(242601);
		expect(backoff.next()).toBe(1000);
	} finally {
		vi.useRealTimers();
	}
});

it("1回の待ちが2分を超えても、待ちが終わった時点から次の2分を測る", () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const backoff = createConnectionBackoff();
		for (let i = 0; i < 12; i++) backoff.next();
		vi.setSystemTime(144000);
		backoff.attemptStarted();
		expect(backoff.next()).toBe(120000);
	} finally {
		vi.useRealTimers();
	}
});
