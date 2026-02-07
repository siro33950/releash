import { describe, expect, it, vi } from "vitest";
import { disableBuiltinDiagnostics } from "../monaco-config";

function createMockMonaco() {
	return {
		languages: {
			typescript: {
				typescriptDefaults: {
					setDiagnosticsOptions: vi.fn(),
				},
				javascriptDefaults: {
					setDiagnosticsOptions: vi.fn(),
				},
			},
		},
	};
}

type MockMonaco = ReturnType<typeof createMockMonaco>;

describe("disableBuiltinDiagnostics", () => {
	it("should disable semantic/suggestion diagnostics and be idempotent", () => {
		const monaco1 = createMockMonaco();
		disableBuiltinDiagnostics(
			monaco1 as unknown as Parameters<typeof disableBuiltinDiagnostics>[0],
		);

		const expectedOptions = {
			noSemanticValidation: true,
			noSyntaxValidation: false,
			noSuggestionDiagnostics: true,
		};

		expect(
			monaco1.languages.typescript.typescriptDefaults.setDiagnosticsOptions,
		).toHaveBeenCalledWith(expectedOptions);
		expect(
			monaco1.languages.typescript.javascriptDefaults.setDiagnosticsOptions,
		).toHaveBeenCalledWith(expectedOptions);

		const monaco2: MockMonaco = createMockMonaco();
		disableBuiltinDiagnostics(
			monaco2 as unknown as Parameters<typeof disableBuiltinDiagnostics>[0],
		);

		expect(
			monaco2.languages.typescript.typescriptDefaults.setDiagnosticsOptions,
		).not.toHaveBeenCalled();
	});
});
