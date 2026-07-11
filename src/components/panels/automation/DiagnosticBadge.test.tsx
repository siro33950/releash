import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { DiagnosticItem } from "@/types/workflow";
import { DiagnosticItemRow } from "./DiagnosticBadge";

function diagnostic(overrides: Partial<DiagnosticItem> = {}): DiagnosticItem {
	return {
		code: "WFT001",
		severity: "error",
		stage: "typecheck",
		message: "when.on field must be boolean",
		...overrides,
	};
}

describe("DiagnosticItemRow", () => {
	it("renders code, formatted stage, and span label when span is present", () => {
		render(
			<DiagnosticItemRow
				item={diagnostic({
					span: {
						start_line: 12,
						start_col: 7,
						end_line: 12,
						end_col: 11,
					},
				})}
			/>,
		);

		expect(screen.getByText("WFT001")).toBeInTheDocument();
		expect(screen.getByText("typecheck")).toBeInTheDocument();
		expect(screen.getByText("12:7")).toBeInTheDocument();
	});

	it("omits span label when span is absent and formats snake_case stage", () => {
		render(
			<DiagnosticItemRow
				item={diagnostic({
					code: "WFR003",
					stage: "parse_shape",
					span: undefined,
				})}
			/>,
		);

		expect(screen.getByText("WFR003")).toBeInTheDocument();
		expect(screen.getByText("parse shape")).toBeInTheDocument();
		expect(screen.queryByText(/^\d+:\d+$/)).not.toBeInTheDocument();
	});
});
