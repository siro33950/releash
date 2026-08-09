import { describe, expect, it } from "vitest";
import { providerAgentSessionIconPresentation } from "./WorkflowNodeStatusIcon";

describe("providerAgentSessionIconPresentation", () => {
	it("open＋runningはworkflow node runningと同じblue＋pulseになる", () => {
		expect(
			providerAgentSessionIconPresentation({
				lifecycle: "open",
				activity: "running",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-blue-600 dark:text-blue-300",
			pulse: true,
			statusLabel: "running",
		});
	});

	it("open＋idleはニュートラル色でpulseしない", () => {
		expect(
			providerAgentSessionIconPresentation({
				lifecycle: "open",
				activity: "idle",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-foreground",
			pulse: false,
			statusLabel: "open",
		});
	});

	it("paused＋異常終了はdestructiveになる", () => {
		expect(
			providerAgentSessionIconPresentation({
				lifecycle: "paused",
				activity: "idle",
				lastExitAbnormal: true,
			}),
		).toEqual({
			className: "text-destructive",
			pulse: false,
			statusLabel: "paused (exited abnormally)",
		});
	});

	it("paused（正常）は非活性のdimになる", () => {
		expect(
			providerAgentSessionIconPresentation({
				lifecycle: "paused",
				activity: "idle",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-muted-foreground",
			pulse: false,
			statusLabel: "paused",
		});
	});

	it("archivedは非活性のdimになる", () => {
		expect(
			providerAgentSessionIconPresentation({
				lifecycle: "archived",
				activity: "idle",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-muted-foreground",
			pulse: false,
			statusLabel: "archived",
		});
	});
});
