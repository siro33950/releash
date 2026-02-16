import { trackEvent as aptabaseTrackEvent } from "@aptabase/tauri";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTelemetryEnabled, trackEvent } from "../telemetry";

describe("telemetry", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		setTelemetryEnabled(true);
	});

	it("enabled の場合、aptabase の trackEvent を呼び出す", () => {
		trackEvent("test_event", { key: "value" });
		expect(aptabaseTrackEvent).toHaveBeenCalledWith("test_event", {
			key: "value",
		});
	});

	it("disabled の場合、aptabase の trackEvent を呼び出さない", () => {
		setTelemetryEnabled(false);
		trackEvent("test_event");
		expect(aptabaseTrackEvent).not.toHaveBeenCalled();
	});

	it("再度 enabled にすると aptabase の trackEvent を呼び出す", () => {
		setTelemetryEnabled(false);
		trackEvent("first");
		expect(aptabaseTrackEvent).not.toHaveBeenCalled();

		setTelemetryEnabled(true);
		trackEvent("second");
		expect(aptabaseTrackEvent).toHaveBeenCalledWith("second", undefined);
	});

	it("props なしで呼び出せる", () => {
		trackEvent("no_props");
		expect(aptabaseTrackEvent).toHaveBeenCalledWith("no_props", undefined);
	});

	it("aptabase がエラーを投げてもクラッシュしない", () => {
		vi.mocked(aptabaseTrackEvent).mockRejectedValueOnce(
			new Error("network error"),
		);
		expect(() => trackEvent("fail_event")).not.toThrow();
	});
});
